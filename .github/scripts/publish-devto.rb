#!/usr/bin/env ruby
# frozen_string_literal: true

require "json"
require "net/http"
require "uri"
require "yaml"

module DevtoPublish
  class Error < StandardError; end

  Article = Struct.new(
    :path, :title, :published, :description, :tags, :cover_image,
    :canonical_url, :devto_id, :body_markdown,
    keyword_init: true
  )

  module_function

  def parse_article(path)
    lines = File.readlines(path, encoding: "UTF-8")
    start_index = lines.index { |line| line.strip == "---" }
    finish_index = start_index && lines.each_index.find do |index|
      index > start_index && lines[index].strip == "---"
    end
    raise Error, "#{path}: missing complete YAML front matter" unless finish_index

    metadata = YAML.safe_load(
      lines[(start_index + 1)...finish_index].join,
      permitted_classes: [],
      permitted_symbols: [],
      aliases: false
    )
    raise Error, "#{path}: front matter must be a mapping" unless metadata.is_a?(Hash)

    title = metadata["title"]
    published = metadata["published"] == true
    if published && (title.nil? || title.to_s.strip.empty?)
      raise Error, "#{path}: published article has no title"
    end

    tags = metadata["tags"]
    tags = tags.split(",") if tags.is_a?(String)
    tags = Array(tags).map { |tag| tag.to_s.strip }.reject(&:empty?)

    devto_id = metadata["devto_id"]
    if devto_id && (!devto_id.is_a?(Integer) || devto_id < 1)
      raise Error, "#{path}: devto_id must be a positive integer"
    end

    Article.new(
      path: path,
      title: title.to_s.strip,
      published: published,
      description: metadata["description"].to_s,
      tags: tags,
      cover_image: metadata["cover_image"].to_s,
      canonical_url: metadata["canonical_url"].to_s,
      devto_id: devto_id,
      body_markdown: lines[(finish_index + 1)..].join.strip
    )
  rescue Psych::Exception => e
    raise Error, "#{path}: invalid YAML front matter: #{e.message}"
  end

  class Client
    DEFAULT_BASE_URL = "https://dev.to/api/"
    DEFAULT_PAGE_SIZE = 1000

    def initialize(api_key:, base_url: DEFAULT_BASE_URL, page_size: DEFAULT_PAGE_SIZE, transport: nil)
      @api_key = api_key
      @base_url = URI(base_url)
      @page_size = page_size
      @transport = transport || method(:http_request)
    end

    def all_articles
      articles = []
      page = 1
      loop do
        batch = request_json(:get, "articles/me?page=#{page}&per_page=#{@page_size}")
        raise Error, "Dev.to articles response is not an array" unless batch.is_a?(Array)

        articles.concat(batch)
        break if batch.length < @page_size

        page += 1
      end
      articles
    end

    def publish(article, id: nil)
      payload = {
        article: {
          title: article.title,
          body_markdown: article.body_markdown,
          published: true,
          tags: article.tags
        }
      }
      optional = {
        description: article.description,
        main_image: article.cover_image,
        canonical_url: article.canonical_url
      }
      optional.each { |key, value| payload[:article][key] = value unless value.empty? }

      method = id ? :put : :post
      path = id ? "articles/#{id}" : "articles"
      response = request_json(method, path, payload)
      unless response.is_a?(Hash) && response["id"].is_a?(Integer) && !response["url"].to_s.empty?
        raise Error, "Dev.to #{method.upcase} response omitted article id or URL"
      end
      response
    end

    private

    def request_json(method, path, payload = nil)
      status, body = @transport.call(method, URI.join(@base_url.to_s, path), payload)
      unless status.between?(200, 299)
        detail = body.to_s.gsub(/\s+/, " ")[0, 500]
        raise Error, "Dev.to API returned HTTP #{status}: #{detail}"
      end
      JSON.parse(body)
    rescue JSON::ParserError => e
      raise Error, "Dev.to API returned invalid JSON: #{e.message}"
    end

    def http_request(method, uri, payload)
      request_class = {get: Net::HTTP::Get, post: Net::HTTP::Post, put: Net::HTTP::Put}.fetch(method)
      request = request_class.new(uri)
      request["api-key"] = @api_key
      request["Content-Type"] = "application/json" if payload
      request.body = JSON.generate(payload) if payload

      response = Net::HTTP.start(
        uri.host,
        uri.port,
        use_ssl: uri.scheme == "https",
        open_timeout: 10,
        read_timeout: 20,
        write_timeout: 20
      ) { |http| http.request(request) }
      [response.code.to_i, response.body.to_s]
    rescue SystemCallError, SocketError, Timeout::Error => e
      raise Error, "Dev.to request failed: #{e.message}"
    end
  end

  class Publisher
    def initialize(client:, output: $stdout)
      @client = client
      @output = output
    end

    def run(paths)
      articles = paths.select { |path| File.file?(path) }.map { |path| DevtoPublish.parse_article(path) }
      raise Error, "no article files found" if articles.empty?

      published = articles.select(&:published)
      validate_unique_articles(published)
      existing = @client.all_articles
      published.each do |article|
        id = article.devto_id || match_existing(existing, article)&.fetch("id")
        @output.puts(id ? "Updating: #{article.title} (id=#{id})" : "Creating: #{article.title}")
        response = @client.publish(article, id: id)
        @output.puts(response.fetch("url"))
        existing << response unless id
      end
    end

    private

    def validate_unique_articles(articles)
      seen = {}
      articles.each do |article|
        key = if article.devto_id
                ["devto_id", article.devto_id]
              elsif !article.canonical_url.empty?
                ["canonical_url", article.canonical_url]
              else
                ["title", article.title]
              end
        previous = seen[key]
        if previous
          raise Error, "#{article.path}: duplicate #{key.first} also used by #{previous}"
        end
        seen[key] = article.path
      end
    end

    def match_existing(existing, article)
      matches = if !article.canonical_url.empty?
                  existing.select { |item| item["canonical_url"] == article.canonical_url }
                else
                  []
                end
      matches = existing.select { |item| item["title"] == article.title } if matches.empty?
      if matches.length > 1
        raise Error, "#{article.path}: multiple Dev.to articles match; add a devto_id to front matter"
      end
      matches.first
    end
  end
end

if $PROGRAM_NAME == __FILE__
  api_key = ENV["DEVTO_API_KEY"].to_s
  if api_key.empty?
    puts "No Dev.to API key configured, skipping"
    exit 0
  end

  begin
    client = DevtoPublish::Client.new(api_key: api_key)
    DevtoPublish::Publisher.new(client: client).run(ARGV)
  rescue DevtoPublish::Error => e
    warn "Dev.to publish failed: #{e.message}"
    exit 1
  end
end
