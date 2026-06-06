#!/usr/bin/env ruby
# Checks local Markdown file links without depending on external network access.

require "pathname"
require "uri"

root = Pathname.pwd
ignored_prefixes = [
  ".docker-config/",
  ".git/",
  "target/"
]

markdown_files = Dir.glob("**/*.md").reject do |path|
  ignored_prefixes.any? { |prefix| path.start_with?(prefix) }
end

errors = []
link_pattern = /(?<!!)\[[^\]]+\]\(([^)\s]+)(?:\s+"[^"]*")?\)/

markdown_files.each do |file|
  content = File.read(file)
  content.scan(link_pattern).flatten.each do |raw_target|
    target = raw_target.delete_prefix("<").delete_suffix(">")
    next if target.match?(/\A(?:https?:|mailto:|tel:)/)
    next if target.start_with?("#")

    file_target = target.sub(/#.*/, "")
    next if file_target.empty?

    decoded_target = URI.decode_www_form_component(file_target)
    target_path = (Pathname.new(file).dirname + decoded_target).cleanpath
    next if File.exist?(root + target_path)

    errors << "#{file}: missing local link target #{target}"
  end
end

if errors.any?
  warn errors.join("\n")
  exit 1
end

puts "Checked #{markdown_files.length} Markdown files."
