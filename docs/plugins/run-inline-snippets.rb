#!/usr/bin/ruby

require "json"
require "shellwords"
require "tmpdir"
require "digest/md5"
require "fileutils"
require "json"
require "open3"

if ARGV[0] == "supports"
    if ARGV[1] == "html"
        exit 0
    else
        exit 1
    end
end

$simulation_data_path = "../simulation/memory-profiling-simulation.dat"
$cli_path = "../target/debug/memory-profiler-cli"

unless File.exist? $simulation_data_path
    system "../simulation/generate-simulation-data.sh 1>&2"
    raise "failed to generate simulation data" unless $?.exitstatus == 0
end

system "cargo build -p memory-profiler-cli 1>&2"
raise "failed to build the CLI" unless $?.exitstatus == 0

raise unless File.exist? $cli_path

$simulation_data_path = File.expand_path $simulation_data_path
$cli_path = File.expand_path $cli_path

def process root, section
    chapter = section["Chapter"]

    cache_root = File.join root, ".cache"
    generated_root = File.join root, "src", "generated"

    FileUtils.mkdir_p cache_root
    FileUtils.mkdir_p generated_root

    content_checksum = Digest::MD5.hexdigest( chapter["content"] )
    content_cache_path = File.join cache_root, content_checksum
    if File.exist? content_cache_path
        content = File.read content_cache_path
    else
        has_code_to_run = false
        output = []
        chapter["content"].split( /^(```.*?\n.+?\n```)/m ).each do |chunk|
            if chunk.start_with?( "```" ) && chunk.end_with?( "```" )
                raise unless chunk =~ /^```(.*?)\n(.+?)```/m
                attrs, code = $1, $2

                run = false
                hide_code = false
                vanilla_attrs = []

                attrs.split( "," ).each do |attr|
                    attr = attr.strip
                    if attr.start_with? "%"
                        if attr == "%run"
                            run = true
                        elsif attr == "%hide-code"
                            hide_code = true
                        else
                            raise "unsupported attribute: '#{attr}'"
                        end
                    else
                        if attr == "rhai"
                            attr = "rust,ignore"
                        end
                        vanilla_attrs << attr
                    end
                end

                unless hide_code
                    code_for_display = []
                    skip = false
                    code.split( "\n" ).each do |line|
                        if skip
                            skip = false
                            next
                        end
                        if line.strip == "// %hide_next_line"
                            skip = true
                            next
                        end
                        code_for_display << line
                    end
                    code_for_display = "```#{vanilla_attrs.join( "," )}\n#{code_for_display.join( "\n" ).strip}\n```\n"
                    output << [:text, code_for_display]
                end

                if run
                    output << [:code, code]
                    has_code_to_run = true
                end
            else
                output << [:text, chunk]
            end
        end

        content = ""
        if has_code_to_run
            stdin, stdout, stderr, wait = Open3.popen3( $cli_path, "script-slave", "--data", $simulation_data_path )
            output_preprocessed = []
            output.each do |kind, chunk|
                if kind == :text
                    output_preprocessed << {
                        "kind" => "text",
                        "chunk" => chunk
                    }
                elsif kind == :code
                    stdin.print chunk
                    stdin.print "\0"
                    stdin.flush
                    loop do
                        line = nil
                        begin
                            line = stdout.readline
                            rescue EOFError
                                STDERR.puts "EOF reached when reading script execution results"
                                STDERR.puts stderr.read
                                exit 1
                        end
                        obj = JSON.parse line

                        case obj["kind"]
                            when "println"
                                obj["message"] += "\n"
                                if output_preprocessed.empty? == false && output_preprocessed[-1]["kind"] == "println"
                                    output_preprocessed[-1]["message"] += obj["message"]
                                else
                                    output_preprocessed << obj
                                end
                            when "image"
                                output_preprocessed << obj
                            when "runtime_error", "syntax_error"
                                STDERR.puts "Error while running '#{chapter["path"]}': #{obj["message"]}"
                                exit 1
                            when "idle"
                                break
                            else
                                raise "unknown message kind: #{obj["kind"]}"
                        end
                    end
                else
                    raise
                end
            end

            output_preprocessed.each do |obj|
                case obj["kind"]
                    when "text"
                        content += obj["chunk"]
                    when "println"
                        content += "\n" if !content.empty?
                        content += "```\n"
                        content += "#{obj["message"]}"
                        content += "```\n\n"
                    when "image"
                        data = obj["data"].pack( "C*" )
                        file_checksum = Digest::MD5.hexdigest( data )
                        target_path = File.join generated_root, "#{file_checksum}.svg"
                        target_filename = "#{file_checksum}.svg"
                        unless File.exist? target_path
                            File.write target_path, data
                        end

                        prefix = ([".."] * chapter["path"].count( "/" )).join( "/" )
                        prefix = "#{prefix}/" unless prefix.empty?

                        content += "\n" unless content.empty?
                        content += "[![](#{prefix}generated/#{target_filename})](#{prefix}generated/#{target_filename})"
                    else
                        raise
                end
            end
        else
            output.each do |kind, chunk|
                raise unless kind == :text
                content += chunk
            end
        end

        File.write content_cache_path, content
    end

    chapter["content"] = content

    if chapter["sub_items"]
        chapter["sub_items"] = chapter["sub_items"].map do |subsection|
            process root, subsection
        end
    end
    section["Chapter"] = chapter
    section
end

context, book = JSON.parse STDIN.read

book["sections"] = book["sections"].map do |section|
    section = process context["root"], section
    section
end

print book.to_json
