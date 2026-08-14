#!/usr/bin/env ruby
# frozen_string_literal: true

require "minitest/autorun"
require "stringio"
require "tempfile"
require_relative "publish-devto"

class DevtoPublishTest < Minitest::Test
  def test_all_repository_articles_parse_with_unique_stable_identifiers
    articles = Dir[File.expand_path("../../articles/*.md", __dir__)].sort.map do |path|
      DevtoPublish.parse_article(path)
    end
    refute_empty articles
    assert articles.all?(&:published)
    assert articles.all? { |article| !article.title.empty? && !article.body_markdown.empty? }

    keys = articles.map do |article|
      article.devto_id ? ["devto_id", article.devto_id] : ["canonical_url", article.canonical_url]
    end
    assert_equal keys.uniq, keys
    assert_equal articles.map(&:title).uniq, articles.map(&:title)
    assert articles.all? { |article| !article.canonical_url.empty? }
  end

  def test_parses_real_yaml_forms_and_body_after_front_matter
    file = Tempfile.new(["article", ".md"])
    file.write(<<~ARTICLE)
      ```markdown
      ---
      title: Unquoted title
      published: true
      description: >-
        Folded description
      tags:
        - rust
        - ai
      canonical_url: https://example.test/article
      ---

      Body text
      ```ruby
      puts "inside"
      ```
      ```
      Trailing wrapper commentary
    ARTICLE
    file.close

    article = DevtoPublish.parse_article(file.path)
    assert_equal "Unquoted title", article.title
    assert_equal "Folded description", article.description
    assert_equal %w[rust ai], article.tags
    assert_equal "Body text\n```ruby\nputs \"inside\"\n```", article.body_markdown
  ensure
    file&.close!
  end

  def test_client_paginates_and_checks_http_and_json_errors
    calls = []
    transport = lambda do |method, uri, _payload|
      calls << [method, uri.path, uri.query]
      page = URI.decode_www_form(uri.query).to_h.fetch("page")
      body = if page == "1"
               JSON.generate([
                 {id: 1, title: "One", canonical_url: "https://example.test/one"},
                 {id: 2, title: "Two", canonical_url: "https://example.test/two"}
               ])
             else
               "[]"
             end
      [200, body]
    end
    client = DevtoPublish::Client.new(api_key: "secret", page_size: 2, transport: transport)
    assert_equal 2, client.all_articles.length
    assert_equal [
      [:get, "/api/articles/me/all", "page=1&per_page=2"],
      [:get, "/api/articles/me/all", "page=2&per_page=2"]
    ], calls

    failing = DevtoPublish::Client.new(
      api_key: "secret", transport: ->(*) { [401, '{"error":"bad key"}'] }
    )
    assert_raises(DevtoPublish::Error) { failing.all_articles }

    malformed = DevtoPublish::Client.new(api_key: "secret", transport: ->(*) { [200, "not json"] })
    assert_raises(DevtoPublish::Error) { malformed.all_articles }

    invalid_inventory = DevtoPublish::Client.new(
      api_key: "secret", transport: ->(*) { [200, '[{"title":"missing id"}]'] }
    )
    assert_raises(DevtoPublish::Error) { invalid_inventory.all_articles }

    invalid_match_fields = DevtoPublish::Client.new(
      api_key: "secret", transport: ->(*) { [200, '[{"id":1,"title":[]}]'] }
    )
    assert_raises(DevtoPublish::Error) { invalid_match_fields.all_articles }

    duplicate_inventory = DevtoPublish::Client.new(
      api_key: "secret", page_size: 2,
      transport: lambda do |_, uri, _|
        page = URI.decode_www_form(uri.query).to_h.fetch("page")
        page == "1" ? [200, '[{"id":1,"title":"One"},{"id":2,"title":"Two"}]'] : [200, '[{"id":2,"title":"Two"}]']
      end
    )
    assert_raises(DevtoPublish::Error) { duplicate_inventory.all_articles }
  end

  def test_publisher_updates_by_canonical_url_and_creates_unknown_article
    client = FakeClient.new([
      {"id" => 7, "title" => "Old title", "canonical_url" => "https://example.test/known"}
    ])
    output = StringIO.new
    publisher = DevtoPublish::Publisher.new(client: client, output: output)

    known = article_file("New title", "https://example.test/known")
    fresh = article_file("Fresh title", "https://example.test/fresh")
    publisher.run([known.path, fresh.path])

    assert_equal [7, nil], client.ids
    assert_includes output.string, "Updating: New title (id=7)"
    assert_includes output.string, "Creating: Fresh title"
  ensure
    known&.close!
    fresh&.close!
  end

  def test_client_uses_post_for_create_and_put_for_update
    calls = []
    transport = lambda do |method, uri, payload|
      calls << [method, uri.path, payload]
      id = method == :post ? 10 : 7
      [200, JSON.generate({id: id, url: "https://dev.to/article-#{id}"})]
    end
    client = DevtoPublish::Client.new(api_key: "secret", transport: transport)
    file = article_file("Article", "https://example.test/article")
    article = DevtoPublish.parse_article(file.path)

    client.publish(article)
    client.publish(article, id: 7)

    assert_equal [:post, "/api/articles"], calls[0][0, 2]
    assert_equal [:put, "/api/articles/7"], calls[1][0, 2]
    assert_equal "Article", calls[0][2].dig(:article, :title)

    wrong_id = DevtoPublish::Client.new(
      api_key: "secret",
      transport: ->(*) { [200, JSON.generate({id: 8, url: "https://dev.to/wrong"})] }
    )
    assert_raises(DevtoPublish::Error) { wrong_id.publish(article, id: 7) }

    invalid_url = DevtoPublish::Client.new(
      api_key: "secret",
      transport: ->(*) { [200, JSON.generate({id: 7, url: ["not", "a", "url"]})] }
    )
    assert_raises(DevtoPublish::Error) { invalid_url.publish(article, id: 7) }
  ensure
    file&.close!
  end

  def test_ambiguous_title_fails_closed
    client = FakeClient.new([
      {"id" => 1, "title" => "Same"},
      {"id" => 2, "title" => "Same"}
    ])
    file = article_file("Same", "")
    publisher = DevtoPublish::Publisher.new(client: client, output: StringIO.new)

    assert_raises(DevtoPublish::Error) { publisher.run([file.path]) }
  ensure
    file&.close!
  end

  def test_duplicate_local_canonical_url_fails_before_api_writes
    client = FakeClient.new([])
    first = article_file("First", "https://example.test/duplicate")
    second = article_file("Second", "https://example.test/duplicate")
    publisher = DevtoPublish::Publisher.new(client: client, output: StringIO.new)

    assert_raises(DevtoPublish::Error) { publisher.run([first.path, second.path]) }
    assert_empty client.ids
  ensure
    first&.close!
    second&.close!
  end

  def test_explicit_id_must_exist_in_authenticated_inventory
    client = FakeClient.new([])
    file = article_file("Known by id", "https://example.test/id", devto_id: 7)
    publisher = DevtoPublish::Publisher.new(client: client, output: StringIO.new)

    assert_raises(DevtoPublish::Error) { publisher.run([file.path]) }
    assert_empty client.ids
  ensure
    file&.close!
  end

  def test_explicit_id_must_identify_the_expected_remote_article
    client = FakeClient.new([
      {"id" => 7, "title" => "Other", "canonical_url" => "https://example.test/other"}
    ])
    file = article_file("Expected", "https://example.test/expected", devto_id: 7)
    publisher = DevtoPublish::Publisher.new(client: client, output: StringIO.new)

    assert_raises(DevtoPublish::Error) { publisher.run([file.path]) }
    assert_empty client.ids
  ensure
    file&.close!
  end

  def test_all_remote_matches_are_preflighted_before_writes
    client = FakeClient.new([
      {"id" => 1, "title" => "Ambiguous"},
      {"id" => 2, "title" => "Ambiguous"}
    ])
    fresh = article_file("Fresh", "https://example.test/fresh")
    ambiguous = article_file("Ambiguous", "")
    publisher = DevtoPublish::Publisher.new(client: client, output: StringIO.new)

    assert_raises(DevtoPublish::Error) { publisher.run([fresh.path, ambiguous.path]) }
    assert_empty client.ids
  ensure
    fresh&.close!
    ambiguous&.close!
  end

  def test_different_local_identifiers_cannot_target_same_remote_article
    client = FakeClient.new([
      {"id" => 7, "title" => "First", "canonical_url" => "https://example.test/shared"}
    ])
    by_id = article_file("First", "https://example.test/first", devto_id: 7)
    by_url = article_file("Second", "https://example.test/shared")
    publisher = DevtoPublish::Publisher.new(client: client, output: StringIO.new)

    assert_raises(DevtoPublish::Error) { publisher.run([by_id.path, by_url.path]) }
    assert_empty client.ids
  ensure
    by_id&.close!
    by_url&.close!
  end

  def test_invalid_local_payloads_fail_before_remote_writes
    invalid_metadata = [
      "published: \"true\"\ntags: rust",
      "published: true\ntags: [one, two, three, four, five]",
      "published: true\ntags: [rust, rust]",
      "published: true\ntags: rust\ndescription: [not, text]",
      "published: true\ntags: rust\ncanonical_url: relative/path"
    ]
    client = FakeClient.new([])
    publisher = DevtoPublish::Publisher.new(client: client, output: StringIO.new)

    invalid_metadata.each do |metadata|
      file = Tempfile.new(["article", ".md"])
      begin
        file.write(<<~ARTICLE)
          ---
          title: Invalid
          #{metadata}
          ---
          Body
        ARTICLE
        file.close
        assert_raises(DevtoPublish::Error) { publisher.run([file.path]) }
      ensure
        file.close!
      end
    end
    assert_empty client.ids
  end

  private

  def article_file(title, canonical_url, devto_id: nil)
    file = Tempfile.new(["article", ".md"])
    file.write(<<~ARTICLE)
      ---
      title: #{title.inspect}
      published: true
      tags: rust, ai
      canonical_url: #{canonical_url}
      #{"devto_id: #{devto_id}" if devto_id}
      ---
      Body
    ARTICLE
    file.close
    file
  end

  class FakeClient
    attr_reader :ids

    def initialize(existing)
      @existing = existing
      @ids = []
      @next_id = 100
    end

    def all_articles
      @existing.dup
    end

    def publish(article, id: nil)
      @ids << id
      result_id = id || @next_id.tap { @next_id += 1 }
      {"id" => result_id, "title" => article.title, "url" => "https://dev.to/#{result_id}"}
    end
  end
end
