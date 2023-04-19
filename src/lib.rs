mod grammar;
mod html_process;
mod markdown;
mod url_utility;

use crate::grammar::{GrammarCheckResult, GrammarChecker};
use crate::html_process::process_html;
use anyhow::{Context, Result};
use log::{error, info, trace};
use markdown::{
    parse_markdown_to_html, parse_markdown_to_plaintext, Heading, ParseMarkdownOptions,
    TextStatistics,
};
use std::{
    cmp,
    collections::HashSet,
    fs::{read_to_string, File, OpenOptions},
    future::Future,
    io::{BufRead, BufReader, Write},
    path::Path,
    pin::Pin,
};

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
    stdout_handle: &mut impl Write,
) {
    for result in results {
        writeln!(stdout_handle, "\nText: {}", result.text())
            .expect("Expected to be able to write to stdout");
        writeln!(stdout_handle, "Rule: {}", result.rule())
            .expect("Expected to be able to write to stdout");
        writeln!(
            stdout_handle,
            "Replacements: {}",
            result.replacements_string()
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
        text[..].rfind(|val: char| val == '.' || val == '\n' || val == '!' || val == '?')
    {
        // last character as a &str
        let last = &text[value..value + 1];
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
                Some(_) => (&text[..value + 1], value + 1),
                None => strip_trailing_sentence_stub(&text[..value]),
            },
            _ => unimplemented!("Should not be possible"),
        }
    } else {
        (text, text.len())
    }
}

type CombinedGrammarCheckChunkResults =
    Result<Vec<GrammarCheckResult>, Box<(dyn std::error::Error)>>;

async fn grammar_check(
    markdown: &str,
    _dictionary: &mut HashSet<String>,
    stdout_handle: &mut impl Write,
) {
    let grammar_checker = GrammarChecker::new(None);
    let mut markdown_options = ParseMarkdownOptions::default();
    markdown_options.disable_code_block_output(true);
    let plain_text = parse_markdown_to_plaintext(markdown, markdown_options);

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
    display_grammar_check_results(&combined_grammar_check_results, stdout_handle);
}

pub fn markdown_to_processed_html(markdown: &str, options: &ParseInputOptions) -> ParseResults {
    match parse_markdown_to_html(markdown) {
        Ok((html_value, headings, statistics_value)) => {
            let raw_html = Some(process_html(
                &html_value,
                options.canonical_root_url.as_deref(),
                options.search_term.as_deref(),
            ));
            let html = format!(
                r##"<!DOCTYPE html>
<html lang="en">
  <head>
      <meta charset="UTF-8" >
      <meta name="viewport" content="width=device-width, initial-scale=1.0" >
      <link rel="icon" href="/favicon.ico" sizes="any" >
      <link rel="icon" href="/icon.svg" type="image/svg+xml" >
      <link rel="apple-touch-icon" href="/apple-touch-icon.png" >
      <link rel="manifest" href="/manifest.webmanifest" >
      <meta name="theme-color" content="#032539" >
      <style>:root{{--max-width-full:100%;--max-width-wrapper:38rem;--spacing-px:0.0625rem;--spacing-px-2:0.125rem;--spacing-0:0;--spacing-1:0.25rem;--spacing-4:1rem;--spacing-6:1.5rem;--spacing-12:3rem;--spacing-16:4rem;--font-family:"Helvetica Neue", Helvetica, "Segoe UI", Arial, freesans,
        sans-serif;--font-weight-normal:400;--font-weight-bold:700;--font-weight-black:900;--font-size-root:18px;--font-size-0:0.9rem;--font-size-1:1.125rem;--font-size-2:1.406rem;--font-size-4:2.197rem;--font-size-5:2.747rem;--font-size-6:3.433rem;--line-height-tight:1.3;--line-height-normal:1.5;--line-height-relaxed:1.75;--colour-heading:hsl(200 7% 8%);--color-heading-black:hsl(0 0% 0%);--colour-text:hsl(207 43% 9%)}}*,:after,:before{{box-sizing:border-box}}*{{margin:0}}html{{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;scroll-behavior:smooth}}@media (prefers-reduced-motion:reduce){{html{{scroll-behavior:auto}}}}body{{display:flex;font:1.125rem/1.5"Helvetica Neue",Helvetica,"Segoe UI",Arial,freesans,sans-serif;font:var(--font-size-1)/var(--line-height-normal) var(--font-family);color:hsl(207 43% 9%);color:var(--colour-text);text-rendering:optimizelegibility}}main{{max-width:38rem;max-width:var(--max-width-wrapper);margin-block:4rem;margin-block:var(--spacing-16);margin-inline:auto}}h1{{font-size:2.747rem;font-size:var(--font-size-5)}}h2{{font-size:2.197rem;font-size:var(--font-size-4)}}h3{{font-size:var(--font-size-3)}}h4{{font-size:1.406rem;font-size:var(--font-size-2)}}h1,h2,h3,h4,h5,h6{{margin:3rem 0 1.5rem;margin:var(--spacing-12) var(--spacing-0) var(--spacing-6);line-height:1.3;line-height:var(--line-height-tight)}}h2,h3,h4,h5,h6{{font-weight:700;font-weight:var(--font-weight-bold);color:hsl(200 7% 8%);color:var(--colour-heading)}}p{{line-height:1.75;line-height:var(--line-height-relaxed);margin:0 0 1rem;margin:var(--spacing-0) var(--spacing-0) var(--spacing-4);padding:0;padding:var(--spacing-0)}}p code{{background-color:#e8f1f4;background-color:var(--colour-theme-3-tint-90);border-radius:.125rem;border-radius:var(--spacing-px-2);padding:.0625rem .25rem;padding:var(--spacing-px) var(--spacing-1);-webkit-box-decoration-break:clone;box-decoration-break:clone}}pre{{margin-top:3rem;margin-top:var(--spacing-12);margin-bottom:4rem;margin-bottom:var(--spacing-16);width:100%;width:var(--max-width-full);max-width:100%;max-width:var(--max-width-full);overflow-x:auto}}.heading-anchor{{display:none}}h2:hover .heading-anchor{{display:inline}}</style>
      <title>Markwrite</title>
      <meta name="description" content="Markwrite document" >
      <meta
        name="robots"
        content="index, follow, max-snippet:-1, max-image-preview:large, max-video-preview:-1"
      >
      <link rel="canonical" href="https://rodneylab.com/" >
    <meta name="theme-color" content="#032539" >
  </head>

  <body>
    <main>
      {}
  </main>
  </body>
</html>"##,
                raw_html.unwrap()
            );
            let headings = Some(headings);
            let statistics = Some(statistics_value);
            ParseResults {
                html: Some(html),
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
    let mut dictionary_file = match OpenOptions::new()
        .append(true)
        .create(true)
        .open(dictionary_path)
    {
        Ok(value) => value,
        Err(_) => {
            writeln!(stdout_handle, "[ INFO ] Unable to create dictionary file.")
                .expect("Expected to be able to write to stdout");
            error!("[ ERROR ] Unable to create the dictionary file!");
            return;
        }
    };

    dictionary_file
        .write_all(new_word.as_bytes())
        .with_context(|| {
            format!("[ ERROR ] Unable to write to dictionary file: {dictionary_display_path}",)
        })
        .unwrap();
}

pub fn load_dictionary<P: AsRef<Path>>(
    dictionary_path: P,
    dictionary: &mut HashSet<String>,
    mut stdout_handle: impl Write,
) {
    let dictionary_file = match File::open(dictionary_path) {
        Ok(value) => value,
        Err(_) => {
            writeln!(stdout_handle, "[ INFO ] no dictionary file found.")
                .expect("Expected to be able to stdout");
            return;
        }
    };

    let reader = BufReader::new(&dictionary_file);
    reader.lines().for_each(|line| {
        if let Ok(word_value) = line {
            dictionary.insert(word_value);
        };
    });
}

fn strip_frontmatter(input: &str) -> &str {
    let mut lines = input.lines();
    if let Some(first_line) = lines.next() {
        if first_line.trim_end() != "---" {
            return input;
        };

        let rest = match input.split_once('\n') {
            Some((_first_line, rest)) => rest,
            None => {
                return input;
            }
        };
        return match rest.split_once("\n---") {
            Some((_frontmatter, body)) => body.trim(),
            None => input,
        };
    }
    input
}

#[derive(Default)]
pub struct MarkwriteOptions {
    check_grammar: bool,
}

impl MarkwriteOptions {
    pub fn check_grammar(&self) -> bool {
        self.check_grammar
    }

    pub fn enable_grammar_check(&mut self) {
        self.check_grammar = true;
    }
}

pub async fn update_html<P1: AsRef<Path>, P2: AsRef<Path>>(
    path: &P1,
    output_path: &P2,
    dictionary: &mut HashSet<String>,
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

    let markdown = strip_frontmatter(&markdown);
    let ParseResults {
        html, statistics, ..
    } = markdown_to_processed_html(markdown, &options);
    let word_count = if let Some(value) = statistics {
        value.word_count()
    } else {
        0
    };

    if markwrite_options.check_grammar() {
        grammar_check(markdown, dictionary, stdout_handle).await;
    }

    let output_display_path = output_path.as_ref().display().to_string();
    match html {
        Some(value) => {
            let mut outfile = match File::create(output_path) {
                Ok(value) => value,
                Err(_) => panic!("[ ERROR ] Unable to create the output file!",),
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
    };
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
    use std::{
        collections::HashSet,
        fs::{self, read_to_string, remove_file},
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
        let result = strip_frontmatter(markdown);

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
        let result = strip_frontmatter(markdown);

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
        let mut dictionary = HashSet::new();
        let markdown_path = Path::new("./fixtures/file.md");
        let html_path = Path::new("./fixtures/file.html");
        let stdout = io::stdout();
        let mut handle = io::BufWriter::new(stdout);
        let options = MarkwriteOptions::default();

        // act
        update_html(
            &markdown_path,
            &html_path,
            &mut dictionary,
            &options,
            &mut handle,
        )
        .await
        .expect("Error calling update_html");

        // assert
        let html = read_to_string(html_path).expect("Unable to read generated HTML");
        let expected_result = "<h1 id=\"test\">Test</h1>\n<p>This is a test.</p>\n";
        assert_eq!(html, expected_result);

        // cleanup
        remove_file(html_path).expect("Unable to delete HTML output in cleanup");
    }
}
