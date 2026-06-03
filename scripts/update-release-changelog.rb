#!/usr/bin/env ruby
# frozen_string_literal: true

require "date"
require "fileutils"
require "open3"

def required_env(name)
  value = ENV[name]
  raise "#{name} is required" if value.nil? || value.empty?

  value
end

def generated_changelog(tag, previous_tag)
  generated_path = ENV["GENERATED_CHANGELOG_PATH"]
  if generated_path && File.file?(generated_path)
    content = File.read(generated_path)
    return content unless content.strip.empty?
  end

  git_cliff = ENV.fetch("GIT_CLIFF_BIN", "git-cliff")
  config = ENV.fetch("GIT_CLIFF_CONFIG", "cliff.toml")
  stdout, stderr, status = Open3.capture3(
    git_cliff,
    "--config",
    config,
    "--tag",
    tag,
    "#{previous_tag}..HEAD"
  )
  return stdout if status.success?

  warn stderr
  raise "git-cliff failed with status #{status.exitstatus}"
end

def normalize_section(raw_section, tag)
  version = tag.delete_prefix("v")
  release_date = ENV.fetch("RELEASE_DATE", Date.today.iso8601)
  heading = "## [#{version}] - #{release_date}"
  section = raw_section.strip

  if section.start_with?("## ")
    section.sub(/\A## \[[^\]]+\](?: - \d{4}-\d{2}-\d{2})?/, heading)
  elsif section.empty?
    "#{heading}\n\nNo notable changes."
  else
    "#{heading}\n\n#{section}"
  end.gsub(/\n{3,}/, "\n\n").gsub(/\n\n(?=- )/, "\n")
end

def update_changelog(changelog_path, section)
  changelog = File.read(changelog_path)
  unreleased = /^## \[Unreleased\]\s*\n.*?(?=^## \[|\z)/m
  replacement = "## [Unreleased]\n\n#{section}\n\n"

  unless changelog.match?(unreleased)
    raise "#{changelog_path} must contain a '## [Unreleased]' section"
  end

  File.write(changelog_path, changelog.sub(unreleased, replacement))
end

tag = required_env("TAG")
previous_tag = required_env("PREVIOUS_TAG")
changelog_path = ENV.fetch("CHANGELOG_PATH", "CHANGELOG.md")
release_notes_path = ENV.fetch("RELEASE_NOTES_PATH", ".release/release-notes.md")

section = normalize_section(generated_changelog(tag, previous_tag), tag)
update_changelog(changelog_path, section)

FileUtils.mkdir_p(File.dirname(release_notes_path))
File.write(release_notes_path, "#{section}\n")
