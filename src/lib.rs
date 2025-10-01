#![warn(clippy::all, clippy::pedantic)]

mod grammar;
mod html_process;
mod inline_html;
mod markdown;
mod url_utility;
mod utilities;

use crate::grammar::{CheckResult as GrammarCheckResult, Checker as GrammarChecker};
use crate::html_process::process_html;
use anyhow::{Context, Result};
use askama::Template;
use log::{error, info, trace};
use markdown::{
    parse_markdown_to_html, parse_markdown_to_plaintext, Heading, ParseMarkdownOptions,
    TextStatistics,
};
use owo_colors::{
    colors::{BrightBlue, BrightCyan, White},
    OwoColorize,
};
use serde::Deserialize;
use std::{
    cmp,
    collections::HashSet,
    fs::{read_to_string, File, OpenOptions},
    future::Future,
    include_bytes,
    io::{BufRead, BufReader, Write},
    path::Path,
    pin::Pin,
};
use yaml_rust2::YamlLoader;

pub struct ParseInputOptions {
    canonical_root_url: Option<String>,
    #[allow(unused)]
    enable_smart_punctuation: Option<bool>,
    search_term: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ParseResults {
    html: Option<String>,
    headings: Option<Vec<Heading>>,
    statistics: Option<TextStatistics>,
    errors: Option<Vec<String>>,
}

fn display_grammar_check_results(
    results: &Vec<GrammarCheckResult>,
    path: &str,
    stdout_handle: &mut impl Write,
) {
    for result in results {
        writeln!(
            stdout_handle,
            "\n\n  * {path} / line {}{}{}:",
            "(".fg::<White>(),
            result.short_message().fg::<BrightCyan>(),
            ")".fg::<White>(),
        )
        .expect("Expected to be able to write to stdout");
        writeln!(stdout_handle, "\n    {}\n", result.context())
            .expect("Expected to be able to write to stdout");
        if let Some(value) = result.replacements_string() {
            writeln!(stdout_handle, "    replacements:\n\n{value}",)
                .expect("Expected to be able to write to stdout");
        }
        writeln!(stdout_handle, "    {}", result.sentence().fg::<White>())
            .expect("Expected to be able to write to stdout");
        writeln!(
            stdout_handle,
            "\n    {}\n\n",
            result.message().fg::<BrightBlue>()
        )
        .expect("Expected to be able to write to stdout");
    }
}

/* Text is trimmed into 1500 character chunks for grammar check.  This function
 * was written to help truncate each chunk, so that the chunk ends with
 * complete sentence or two new line characters.
 */
fn strip_trailing_sentence_stub(text: &str) -> (&str, usize) {
    let end = text.len();

    // early return for trivial cases
    if end <= 1 {
        return (text, end);
    }

    if let Some(value) =
        //text[..].rfind(|val: char| val == '.' || val == '\n' || val == '!' || val == '?')
        text[..].rfind(['.', '\n', '!', '?'])
    {
        // last character as a &str
        let last = &text[value..=value];
        if value == end - 1 && last != "\n" {
            return strip_trailing_sentence_stub(&text[..end - 1]);
        }

        // no point trimming right back to the start of the string, so just send everything
        if value == 0 {
            return (text, end);
        }

        match last {
            /* Could be the end of a sentence, check following character is a
             * whitespace character to avoid accidently splitting 10.1, for
             * example.
             */
            "." | "!" | "?" => match &text[value + 1..value + 2].find(char::is_whitespace) {
                Some(_) => (&text[..value + 2], value + 2),
                None => strip_trailing_sentence_stub(&text[..value]),
            },
            "\n" => match &text[value - 1..value].find('\n') {
                Some(_) => (&text[..=value], value + 1),
                None => strip_trailing_sentence_stub(&text[..value]),
            },
            _ => unreachable!("Should not be possible"),
        }
    } else {
        (text, text.len())
    }
}

type CombinedGrammarCheckChunkResults = Result<Vec<GrammarCheckResult>, Box<dyn std::error::Error>>;

async fn grammar_check(markdown: &str, path: &str, stdout_handle: &mut impl Write) {
    let grammar_checker = GrammarChecker::new(None);
    let mut markdown_options = ParseMarkdownOptions::default();
    markdown_options.disable_code_block_output(true);
    let plain_text = parse_markdown_to_plaintext(markdown, &markdown_options);

    let mut start: usize = 0;
    let chunk_size = 1500;
    let plain_text_length = plain_text.len();
    let mut end: usize = cmp::min(plain_text_length, chunk_size);
    let mut result_futures_vec: Vec<Box<dyn Future<Output = CombinedGrammarCheckChunkResults>>> =
        vec![];

    writeln!(
        stdout_handle,
        "[ INFO ] Checking text spelling, punctuation and grammar..."
    )
    .expect("Expected to be able to write to stdout");

    while start < plain_text_length {
        let (chunk, trimmed_chunk_end) = strip_trailing_sentence_stub(&plain_text[start..end]);
        trace!(
            "Chunk: {chunk}\nlines: {}, characters: {}",
            chunk.split('\n').collect::<Vec<&str>>().len(),
            chunk.len()
        );
        let chunk_results = grammar_checker.check_chunk(chunk);
        result_futures_vec.push(Box::new(chunk_results));

        start += trimmed_chunk_end;
        end = cmp::min(plain_text_length, start + chunk_size);
        stdout_handle.flush().expect("Unable to flush to stdout");
    }
    let mut combined_grammar_check_results: Vec<GrammarCheckResult> = Vec::new();
    for result in result_futures_vec {
        let result_values = Pin::from(result).await;
        if let Ok(mut value) = result_values {
            combined_grammar_check_results.append(&mut value);
        }
    }
    display_grammar_check_results(&combined_grammar_check_results, path, stdout_handle);
}

#[derive(Deserialize, PartialEq, Debug)]
pub struct Frontmatter {
    title: Option<String>,
    description: Option<String>,
    canonical_url: Option<String>,
}

#[derive(Template)]
#[template(path = "template.html")]
struct HtmlTemplate<'a> {
    canonical_url: Option<&'a str>,
    description: Option<&'a str>,
    global_css: &'a str,
    language: &'a str,
    live_reload_script: &'a str,
    main_section_html: &'a str,
    prism_dark_theme_css: &'a str,
    prism_light_theme_css: &'a str,
    prism_script: &'a str,
    theme_script: &'a str,
    title: &'a str,
}

fn html_document(main_section_html: &str, frontmatter: &Frontmatter) -> String {
    let language = "en";
    let Frontmatter {
        canonical_url,
        description,
        title,
    } = frontmatter;
    let live_reload_script = &String::from_utf8_lossy(include_bytes!("./resources/live_reload.js"));
    let prism_dark_theme_css =
        &String::from_utf8_lossy(include_bytes!("./resources/prism-one-dark.css"));
    let prism_light_theme_css =
        &String::from_utf8_lossy(include_bytes!("./resources/prism-one-light.css"));
    let prism_script = &String::from_utf8_lossy(include_bytes!("./resources/prism.js"));
    let global_css = &String::from_utf8_lossy(include_bytes!("./resources/styles.css"));
    let theme_script = &String::from_utf8_lossy(include_bytes!("./resources/theme.js"));
    let title = match title {
        Some(value) => value,
        None => "Markwrite Document",
    };

    let html = HtmlTemplate {
        canonical_url: canonical_url.as_deref(),
        description: description.as_deref(),
        global_css,
        language,
        live_reload_script,
        main_section_html,
        prism_dark_theme_css,
        prism_light_theme_css,
        prism_script,
        theme_script,
        title,
    };
    html.render().unwrap()
}

#[must_use]
pub fn markdown_to_processed_html(
    markdown: &str,
    frontmatter: &Frontmatter,
    options: &ParseInputOptions,
) -> ParseResults {
    match parse_markdown_to_html(markdown) {
        Ok((html_value, headings, statistics_value)) => {
            let main_section_html = process_html(
                &html_value,
                options.canonical_root_url.as_deref(),
                options.search_term.as_deref(),
            );
            let html = Some(html_document(&main_section_html, frontmatter));
            let headings = Some(headings);
            let statistics = Some(statistics_value);
            ParseResults {
                html,
                headings,
                statistics,
                errors: None,
            }
        }
        Err(error) => {
            let message = format!("Error parsing markdown: {error}");
            let errors = vec![message];
            ParseResults {
                html: None,
                headings: None,
                statistics: None,
                errors: Some(errors),
            }
        }
    }
}

#[allow(dead_code)]
fn add_word_to_dictionary<P: AsRef<Path>>(
    new_word: &str,
    dictionary_path: P,
    dictionary: &mut HashSet<String>,
    mut stdout_handle: impl Write,
) {
    let dictionary_display_path = dictionary_path.as_ref().display().to_string();
    dictionary.insert(new_word.to_string());
    let Ok(mut dictionary_file) = OpenOptions::new()
        .append(true)
        .create(true)
        .open(dictionary_path)
    else {
        writeln!(stdout_handle, "[ INFO ] Unable to create dictionary file.")
            .expect("Expected to be able to write to stdout");
        error!("[ ERROR ] Unable to create the dictionary file!");
        return;
    };

    dictionary_file
        .write_all(new_word.as_bytes())
        .with_context(|| {
            format!("[ ERROR ] Unable to write to dictionary file: {dictionary_display_path}",)
        })
        .unwrap();
}

pub fn load_dictionary<P: AsRef<Path>, S: ::std::hash::BuildHasher>(
    dictionary_path: P,
    dictionary: &mut HashSet<String, S>,
    mut stdout_handle: impl Write,
) {
    let Ok(dictionary_file) = File::open(dictionary_path) else {
        writeln!(stdout_handle, "[ INFO ] no dictionary file found.")
            .expect("Expected to be able to stdout");
        return;
    };

    let reader = BufReader::new(&dictionary_file);
    reader.lines().for_each(|line| {
        if let Ok(word_value) = line {
            dictionary.insert(word_value);
        }
    });
}

fn strip_frontmatter(input: &str) -> (Option<&str>, &str) {
    let mut lines = input.lines();
    if let Some(first_line) = lines.next() {
        if first_line.trim_end() != "---" {
            return (None, input);
        }

        let Some((_first_line, rest)) = input.split_once('\n') else {
            return (None, input);
        };
        return match rest.split_once("\n---") {
            Some((frontmatter, body)) => (Some(frontmatter.trim()), body.trim()),
            None => (None, input),
        };
    }
    (None, input)
}

#[derive(Default)]
pub struct MarkwriteOptions {
    check_grammar: bool,
}

impl MarkwriteOptions {
    #[must_use]
    pub fn check_grammar(&self) -> bool {
        self.check_grammar
    }

    pub fn enable_grammar_check(&mut self) {
        self.check_grammar = true;
    }
}

///
/// # Errors
/// Errors if unable to read input file
/// # Panics
/// Panics if output path cannot be created
pub async fn update_html<P1: AsRef<Path>, P2: AsRef<Path>>(
    path: &P1,
    output_path: &P2,
    markwrite_options: &MarkwriteOptions,
    stdout_handle: &mut impl Write,
) -> Result<(), notify::Error> {
    let options = ParseInputOptions {
        canonical_root_url: None,
        enable_smart_punctuation: Some(true),
        search_term: None,
    };
    let markdown = match read_to_string(path) {
        Ok(value) => value,
        Err(error) => return Err(error.into()),
    };

    let (frontmatter_yaml, markdown) = strip_frontmatter(&markdown);
    let frontmatter = match frontmatter_yaml {
        Some(value) => match YamlLoader::load_from_str(value) {
            Ok(frontmatter_value) => {
                let doc = &frontmatter_value[0];
                let title = doc["title"].as_str().map(std::string::ToString::to_string);
                let description = doc["description"]
                    .as_str()
                    .map(std::string::ToString::to_string);
                let canonical_url = doc["canonical_url"]
                    .as_str()
                    .map(std::string::ToString::to_string);
                Frontmatter {
                    title,
                    description,
                    canonical_url,
                }
            }
            Err(_) => Frontmatter {
                title: None,
                description: None,
                canonical_url: None,
            },
        },
        None => Frontmatter {
            title: None,
            description: None,
            canonical_url: None,
        },
    };
    let ParseResults {
        html, statistics, ..
    } = markdown_to_processed_html(markdown, &frontmatter, &options);
    let word_count = if let Some(value) = statistics {
        value.word_count()
    } else {
        0
    };

    let display_path = path.as_ref().display().to_string();
    if markwrite_options.check_grammar() {
        grammar_check(markdown, &display_path, stdout_handle).await;
    }

    let output_display_path = output_path.as_ref().display().to_string();
    match html {
        Some(value) => {
            let Ok(mut outfile) = File::create(output_path) else {
                panic!("[ ERROR ] Unable to create the output file!");
            };
            outfile
                .write_all(value.as_bytes())
                .with_context(|| {
                    format!("[ ERROR ] Unable to write to output file: {output_display_path}")
                })
                .unwrap();
            info!("Wrote {output_display_path}.");
            writeln!(
                stdout_handle,
                "[ INFO ] Wrote {output_display_path} ({word_count} words)."
            )?;
        }
        None => eprintln!("[ ERROR ] Unable to parse markdownto HTML"),
    }
    stdout_handle.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        add_word_to_dictionary, load_dictionary, strip_frontmatter, strip_trailing_sentence_stub,
        update_html, MarkwriteOptions,
    };
    use fake::{faker, Fake};
    use html5ever::{
        driver::ParseOpts,
        local_name, namespace_url, ns, parse_document,
        tendril::{fmt::UTF8, Tendril, TendrilSink},
        QualName,
    };
    use markup5ever_rcdom::{NodeData, RcDom};
    use std::{
        collections::HashSet,
        fs::{self, read_to_string, remove_file, File},
        io::{self, BufWriter},
        path::Path,
    };

    #[test]
    fn strip_frontmatter_removes_frontmatter() {
        // arrange
        let markdown = "---
title: Test Document
---

# Test

This is a test.";

        // act
        let (_, result) = strip_frontmatter(markdown);

        // assert
        let expected_result = "# Test

This is a test.";
        assert_eq!(result, expected_result);
    }
    #[test]
    fn add_word_to_dictionary_inserts_new_word() {
        // arrange
        let mut dictionary: HashSet<String> = HashSet::new();
        let stdout = io::stdout();
        let handle = BufWriter::new(stdout);
        let dictionary_path = "fixtures/custom.dict";
        let dictionary_file = assert_fs::NamedTempFile::new("custom.dict")
            .expect("Error getting temp dictionary path");
        let temp_dictionary_path = dictionary_file.path();
        fs::copy(dictionary_path, temp_dictionary_path).expect("Error copying temp fixture");

        // act
        let new_word = "stairs";
        add_word_to_dictionary(new_word, temp_dictionary_path, &mut dictionary, handle);

        // assert
        assert!(dictionary.contains(new_word));
    }

    #[test]
    fn add_word_to_dictionary_updates_dictionary_file() {
        // arrange
        let mut dictionary: HashSet<String> = HashSet::new();
        let stdout = io::stdout();
        let handle = BufWriter::new(stdout);
        let dictionary_path = "fixtures/custom.dict";
        let dictionary_file = assert_fs::NamedTempFile::new("custom.dict")
            .expect("Error getting temp dictionary path");
        let temp_dictionary_path = dictionary_file.path();
        fs::copy(dictionary_path, temp_dictionary_path).expect("Error copying temp fixture");

        // act
        let new_word = "stairs";
        add_word_to_dictionary(new_word, temp_dictionary_path, &mut dictionary, handle);

        // assert
        let dictionary_file_contents =
            read_to_string(temp_dictionary_path).expect("Failed to read file to string");
        assert!(dictionary_file_contents.contains(new_word));
    }

    #[test]
    fn load_dictionary_returns_input_dictionary_when_dictionary_file_is_absent() {
        //arrange
        let mut dictionary: HashSet<String> = HashSet::new();
        let stdout = io::stdout();
        let handle = BufWriter::new(stdout);

        // act
        load_dictionary("nonsense.dict", &mut dictionary, handle);

        //assert
        assert_eq!(dictionary.len(), 0);
    }

    #[test]
    fn load_dictionary_adds_words_from_file_to_dictionary() {
        //arrange
        let mut dictionary: HashSet<String> = HashSet::new();
        let stdout = io::stdout();
        let handle = io::BufWriter::new(stdout);

        // act
        load_dictionary("fixtures/custom.dict", &mut dictionary, handle);

        //assert
        assert_eq!(dictionary.len(), 3);
        assert!(dictionary.contains("Cheese"));
        assert!(dictionary.contains("apples"));
    }

    #[test]
    fn strip_frontmatter_returns_expected_result_when_frontmatter_is_absent() {
        // arrange
        let markdown = "# Test

This is a test.";

        // act
        let (_, result) = strip_frontmatter(markdown);

        // assert
        assert_eq!(result, markdown);
    }

    #[test]
    fn strip_trailing_sentencte_stub_truncates_long_text_chunk() {
        // arrange
        let paragraphs: Vec<String> = faker::lorem::en::Paragraphs(3..5).fake();
        let text = paragraphs.join("\n\n");

        // act
        let (text_chunk, length) = strip_trailing_sentence_stub(&text);

        // asert
        assert!(length <= 1500);
        assert!(text_chunk.len() == length);
        let last = &text_chunk[length - 1..];
        dbg!("LAST: {last}");
        assert!(last == "." || last == "!" || last == "\n" || last == "?");
    }

    #[quickcheck_macros::quickcheck]
    fn strip_trailing_sentencte_stub_truncates_long_text_as_expected() -> bool {
        // arrange
        let paragraphs: Vec<String> = faker::lorem::en::Paragraphs(3..5).fake();
        let text = paragraphs.join("\n\n");

        // act
        let (text_chunk, length) = strip_trailing_sentence_stub(&text);

        // asert
        let last = &text_chunk[length - 1..];
        length <= 1500 && (last == "." || last == "!" || last == "\n" || last == "?")
    }

    #[tokio::test]
    async fn update_html_writes_parsed_markdown_to_html_file() {
        // arrange
        let markdown_path = Path::new("./fixtures/file.md");
        let html_path = Path::new("./fixtures/file_a.html");
        let stdout = io::stdout();
        let mut handle = io::BufWriter::new(stdout);
        let options = MarkwriteOptions::default();

        // act
        update_html(&markdown_path, &html_path, &options, &mut handle)
            .await
            .expect("Error calling update_html");

        // assert
        let mut html_file = File::open(html_path).unwrap();
        let parse_result = parse_document(RcDom::default(), ParseOpts::default())
            .from_utf8()
            .read_from(&mut html_file)
            .expect("Error parsing generated HTML file");
        assert_eq!(parse_result.errors.len(), 0);

        // cleanup
        remove_file(html_path).expect("Unable to delete HTML output in cleanup");
    }

    #[tokio::test]
    async fn update_html_output_has_expected_tags_set() {
        // arrange
        let markdown_path = Path::new("./fixtures/file.md");
        let html_path = Path::new("./fixtures/file_b.html");
        let stdout = io::stdout();
        let mut handle = io::BufWriter::new(stdout);
        let options = MarkwriteOptions::default();

        // act
        update_html(&markdown_path, &html_path, &options, &mut handle)
            .await
            .expect("Error calling update_html");

        // assert
        let mut html_file = File::open(html_path).unwrap();
        let parse_result = parse_document(RcDom::default(), ParseOpts::default())
            .from_utf8()
            .read_from(&mut html_file)
            .expect("Error parsing generated HTML file");

        let document_nodes = parse_result.document.children.borrow();
        let mut document_nodes_iterator = document_nodes.iter();
        let doctype_node = document_nodes_iterator.next().unwrap();

        // check DOCTYPE element
        let NodeData::Doctype { ref name, .. } = doctype_node.data else {
            unimplemented!("Expected DOCTYPE element to exist")
        };
        assert_eq!(name, &Tendril::<UTF8>::from_slice("html"));

        // check html element
        let html_node = document_nodes_iterator.next().unwrap();
        let NodeData::Element {
            ref attrs,
            ref name,
            ..
        } = html_node.data
        else {
            unimplemented!("Expected html element to exist")
        };
        assert_eq!(name, &QualName::new(None, ns!(html), local_name!("html")));

        // check html lang is set
        let mut attrs = attrs.borrow_mut();
        let lang = match attrs.iter_mut().find(|val| &*val.name.local == "lang") {
            Some(value) => &value.value,
            None => unimplemented!("Expected lang attribute to be set on html element"),
        };
        assert_eq!(lang, &Tendril::<UTF8>::from_slice("en"));

        // cleanup
        remove_file(html_path).expect("Unable to delete HTML output in cleanup");
    }
}
