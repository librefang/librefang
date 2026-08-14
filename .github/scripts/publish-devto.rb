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
    unless title.nil? || title.is_a?(String)
      raise Error, "#{path}: title must be a string"
    end
    published_value = metadata["published"]
    unless published_value == true || published_value == false
      raise Error, "#{path}: published must be a boolean"
    end
    published = published_value
    if published && (title.nil? || title.strip.empty?)
      raise Error, "#{path}: published article has no title"
    end

    tags = metadata["tags"]
    tags = tags.split(",") if tags.is_a?(String)
    tags = [] if tags.nil?
    raise Error, "#{path}: tags must be a string or array" unless tags.is_a?(Array)
    unless tags.all? { |tag| tag.is_a?(String) }
      raise Error, "#{path}: every tag must be a string"
    end
    tags = tags.map(&:strip).reject(&:empty?)
    raise Error, "#{path}: articles support at most four tags" if tags.length > 4
    raise Error, "#{path}: tags must be unique" if tags.uniq.length != tags.length

    optional_strings = %w[description cover_image canonical_url].to_h do |field|
      value = metadata[field]
      unless value.nil? || value.is_a?(String)
        raise Error, "#{path}: #{field} must be a string"
      end
      [field, value.to_s]
    end
    %w[cover_image canonical_url].each do |field|
      value = optional_strings.fetch(field)
      next if value.empty?

      uri = URI.parse(value)
      unless %w[http https].include?(uri.scheme) && !uri.host.to_s.empty?
        raise Error, "#{path}: #{field} must be an absolute HTTP(S) URL"
      end
    rescue URI::InvalidURIError
      raise Error, "#{path}: #{field} must be an absolute HTTP(S) URL"
    end

    devto_id = metadata["devto_id"]
    if devto_id && (!devto_id.is_a?(Integer) || devto_id < 1)
      raise Error, "#{path}: devto_id must be a positive integer"
    end

    wrapped = start_index.positive? && lines[start_index - 1].strip.match?(/\A```(?:markdown|md)?\z/i)
    body_end = lines.length
    if wrapped
      closing_index = (finish_index + 1...lines.length).reverse_each.find do |index|
        lines[index].strip == "```"
      end
      raise Error, "#{path}: missing closing Markdown fence" unless closing_index

      body_end = closing_index
    end
    body_markdown = lines[(finish_index + 1)...body_end].join.strip
    raise Error, "#{path}: published article has no body" if published && body_markdown.empty?

    Article.new(
      path: path,
      title: title.to_s.strip,
      published: published,
      description: optional_strings.fetch("description"),
      tags: tags,
      cover_image: optional_strings.fetch("cover_image"),
      canonical_url: optional_strings.fetch("canonical_url"),
      devto_id: devto_id,
      body_markdown: body_markdown
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
        batch = request_json(:get, "articles/me/all?page=#{page}&per_page=#{@page_size}")
        raise Error, "Dev.to articles response is not an array" unless batch.is_a?(Array)
        unless batch.all? do |item|
            item.is_a?(Hash) &&
            item["id"].is_a?(Integer) && item["id"].positive? &&
            item["title"].is_a?(String) &&
            (item["canonical_url"].nil? || item["canonical_url"].is_a?(String))
        end
          raise Error, "Dev.to articles response contains an invalid article"
        end

        articles.concat(batch)
        break if batch.length < @page_size

        page += 1
      end
      if articles.map { |item| item["id"] }.uniq.length != articles.length
        raise Error, "Dev.to paginated inventory contains duplicate article ids"
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
      valid_id = response.is_a?(Hash) && response["id"].is_a?(Integer) && response["id"].positive?
      valid_id &&= response["id"] == id if id
      valid_url = response.is_a?(Hash) && response["url"].is_a?(String) && !response["url"].strip.empty?
      unless valid_id && valid_url
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
      plans = published.map { |article| [article, resolve_existing_id(existing, article)] }
      validate_unique_targets(plans)

      plans.each do |article, id|
        @output.puts(id ? "Updating: #{article.title} (id=#{id})" : "Creating: #{article.title}")
        response = @client.publish(article, id: id)
        @output.puts(response.fetch("url"))
      end
    end

    private

    def validate_unique_articles(articles)
      seen = Hash.new { |hash, key| hash[key] = {} }
      articles.each do |article|
        identifiers = [["title", article.title]]
        identifiers << ["devto_id", article.devto_id] if article.devto_id
        identifiers << ["canonical_url", article.canonical_url] unless article.canonical_url.empty?
        identifiers.each do |kind, value|
          previous = seen[kind][value]
          if previous
            raise Error, "#{article.path}: duplicate #{kind} also used by #{previous}"
          end
          seen[kind][value] = article.path
        end
      end
    end

    def resolve_existing_id(existing, article)
      if article.devto_id
        matches = existing.select { |item| item["id"] == article.devto_id }
        if matches.empty?
          raise Error, "#{article.path}: devto_id #{article.devto_id} is not in the authenticated account"
        end
        if matches.length > 1
          raise Error, "#{article.path}: devto_id #{article.devto_id} is duplicated in remote inventory"
        end
        remote = matches.first
        same_title = remote["title"] == article.title
        same_canonical = !article.canonical_url.empty? && remote["canonical_url"] == article.canonical_url
        unless same_title || same_canonical
          raise Error, "#{article.path}: devto_id #{article.devto_id} matches a different remote article"
        end
        return article.devto_id
      end

      match_existing(existing, article)&.fetch("id")
    end

    def validate_unique_targets(plans)
      seen = {}
      plans.each do |article, id|
        next unless id

        previous = seen[id]
        if previous
          raise Error, "#{article.path}: resolves to Dev.to id #{id}, already used by #{previous}"
        end
        seen[id] = article.path
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
