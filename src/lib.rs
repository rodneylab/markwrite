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
use serde::Deserialize;
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

#[derive(Deserialize, PartialEq, Debug)]
pub struct Frontmatter {
    title: Option<String>,
    description: Option<String>,
    canonical_url: Option<String>,
}

fn html_document(main_section_html: &str, frontmatter: &Frontmatter) -> String {
    let language = "en";
    let Frontmatter {
        canonical_url,
        description,
        title,
    } = frontmatter;

    let head_start = format!(
        r##"<!DOCTYPE html>
<html lang="{language}">
  <head>
      <meta charset="UTF-8" >
      <meta name="viewport" content="width=device-width, initial-scale=1.0" >
      <link rel="icon" href="data:image/x-icon;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAMAAABEpIrGAAAACXBIWXMAAAEuAAABLgF7cRpNAAAAGXRFWHRTb2Z0d2FyZQB3d3cuaW5rc2NhcGUub3Jnm+48GgAAAMlQTFRFHHaPHHaPHXePHneQH3iQIXmSInqSI3qTJXuTJ3yUK3+WLYCXL4GYMYOaM4SaNYWbN4acOYidQo2iSpKmS5KmUpeqWpyuXp6vYJ+wYaCxZqOzaKS0bae3bqe3dKu6eK28e6++hbXDhrbDh7bEiLfEirnFi7nGkLzIncTPoMbQocbQosfRp8rUrM3Wrc7XstDZs9HatdLbudXcwtrhx93jyd7k0+Xp1ubr2unt2+rt4u3x5/Dz6vL07/X38fb48/f5+/z9/v7/////WdYCwAAAAAF0Uk5T/hrjB30AAAC8SURBVDjLzdPHEoJADAZgVlAEGyr2gooNbNh7+9//ocwwjqclXsnpn53vkGR3FUXwpcQImEFYC9+tpqQgj1/dx0keAFtdDhzTzJW7Z0ojOeiEyTgC1wQDRJtigQM2xRIHasA7w4ElcGCaVIeUWnKwGgzc2YWC92eTvQQLNkbkXTimNaUJmpGAmlR3wMNigCg+gb3GANGl0OeAWAMvmwPZG3BKc6tuUPQ5IOY0a132aCvfM30SBJ4Ww58VWR+3BzKDC1fSbwAAAABJRU5ErkJggg==" sizes="any" >
      <link rel="icon" type="image/svg+xml"
      href="data:image/svg+xml,%3C%3Fxml version='1.0' encoding='UTF-8'%3F%3E%3Csvg width='400' height='400' version='1.1' viewBox='0 0 105.83 105.83' xmlns='http://www.w3.org/2000/svg' xmlns:cc='http://creativecommons.org/ns%23' xmlns:dc='http://purl.org/dc/elements/1.1/' xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns%23'%3E%3Cmetadata%3E%3Crdf:RDF%3E%3Ccc:Work rdf:about=''%3E%3Cdc:format%3Eimage/svg+xml%3C/dc:format%3E%3Cdc:type rdf:resource='http://purl.org/dc/dcmitype/StillImage'/%3E%3Cdc:title/%3E%3C/cc:Work%3E%3C/rdf:RDF%3E%3C/metadata%3E%3Crect x='1.7013' y='1.6799' width='102.47' height='102.47' fill='%231c768f' stroke='%231c768f' stroke-width='3.3641'/%3E%3Cg transform='matrix(2.6253 0 0 2.6253 -51.363 -97.03)' fill='%23fff' opacity='.998' style='font-variant-caps:normal;font-variant-east-asian:normal;font-variant-ligatures:normal;font-variant-numeric:normal' aria-label='R'%3E%3Cpath d='m37.305 56.556q1.4911 0 2.6094-0.35413 1.1183-0.37277 1.8638-1.0251t1.1183-1.547q0.37277-0.91328 0.37277-2.013 0-2.1993-1.4538-3.3549t-4.3987-1.1556h-3.5413v9.4497zm12.637 13.979h-3.8954q-1.1556 0-1.6775-0.89464l-6.2625-9.0396q-0.31685-0.46596-0.68962-0.67098t-1.1183-0.20502h-2.423v10.81h-4.3614v-26.839h7.9027q2.6467 0 4.5478 0.54052 1.9198 0.54052 3.1499 1.547 1.2301 0.98784 1.8079 2.3857 0.59643 1.3979 0.59643 3.1126 0 1.3979-0.42868 2.6094-0.41005 1.2115-1.2115 2.1993-0.78282 0.98784-1.9384 1.7147-1.1556 0.7269-2.628 1.1369 0.80145 0.4846 1.3792 1.3606z' fill='%23fff' style='font-variant-caps:normal;font-variant-east-asian:normal;font-variant-ligatures:normal;font-variant-numeric:normal'/%3E%3C/g%3E%3C/svg%3E" />
      <link rel="apple-touch-icon" href="data:image/png;base64,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" >
      <meta name="theme-color" content="#032539" >
      <style>:root{{--max-width-full:100%;--max-width-wrapper:38rem;--spacing-px:0.0625rem;--spacing-px-2:0.125rem;--spacing-0:0;--spacing-1:0.25rem;--spacing-4:1rem;--spacing-6:1.5rem;--spacing-12:3rem;--spacing-16:4rem;--font-family:"Helvetica Neue", Helvetica, "Segoe UI", Arial, freesans,
        sans-serif;--font-weight-normal:400;--font-weight-bold:700;--font-weight-black:900;--font-size-root:18px;--font-size-0:0.9rem;--font-size-1:1.125rem;--font-size-2:1.406rem;--font-size-4:2.197rem;--font-size-5:2.747rem;--font-size-6:3.433rem;--line-height-tight:1.3;--line-height-normal:1.5;--line-height-relaxed:1.75;--colour-heading:hsl(200 7% 8%);--color-heading-black:hsl(0 0% 0%);--colour-text:hsl(207 43% 9%)}}*,:after,:before{{box-sizing:border-box}}*{{margin:0}}html{{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;scroll-behavior:smooth}}@media (prefers-reduced-motion:reduce){{html{{scroll-behavior:auto}}}}body{{display:flex;font:1.125rem/1.5"Helvetica Neue",Helvetica,"Segoe UI",Arial,freesans,sans-serif;font:var(--font-size-1)/var(--line-height-normal) var(--font-family);color:hsl(207 43% 9%);color:var(--colour-text);text-rendering:optimizelegibility}}main{{max-width:38rem;max-width:var(--max-width-wrapper);margin-block:4rem;margin-block:var(--spacing-16);margin-inline:auto}}h1{{font-size:2.747rem;font-size:var(--font-size-5)}}h2{{font-size:2.197rem;font-size:var(--font-size-4)}}h3{{font-size:var(--font-size-3)}}h4{{font-size:1.406rem;font-size:var(--font-size-2)}}h1,h2,h3,h4,h5,h6{{margin:3rem 0 1.5rem;margin:var(--spacing-12) var(--spacing-0) var(--spacing-6);line-height:1.3;line-height:var(--line-height-tight)}}h2,h3,h4,h5,h6{{font-weight:700;font-weight:var(--font-weight-bold);color:hsl(200 7% 8%);color:var(--colour-heading)}}p{{line-height:1.75;line-height:var(--line-height-relaxed);margin:0 0 1rem;margin:var(--spacing-0) var(--spacing-0) var(--spacing-4);padding:0;padding:var(--spacing-0)}}p code{{background-color:#e8f1f4;background-color:var(--colour-theme-3-tint-90);border-radius:.125rem;border-radius:var(--spacing-px-2);padding:.0625rem .25rem;padding:var(--spacing-px) var(--spacing-1);-webkit-box-decoration-break:clone;box-decoration-break:clone}}pre{{margin-top:3rem;margin-top:var(--spacing-12);margin-bottom:4rem;margin-bottom:var(--spacing-16);width:100%;width:var(--max-width-full);max-width:100%;max-width:var(--max-width-full);overflow-x:auto}}.heading-anchor{{display:none}}h2:hover .heading-anchor{{display:inline}}</style> "##
    );
    let title_meta = match title {
        Some(value) => format!("<title>{value}</title>"),
        None => "<title>Markwrite Document</title>".to_string(),
    };
    let description_meta = match description {
        Some(value) => format!("<meta name=\"description\" content=\"{value}\" >\n"),
        None => String::new(),
    };
    let canonical_meta = match canonical_url {
        Some(value) => format!("<link rel=\"canonical\" href=\"{value}\" >\n"),
        None => String::new(),
    };

    format!(
        "{head_start}\n{title_meta}\n{description_meta}\n{canonical_meta}
  </head>

  <body>
    <main>
      {main_section_html}
  </main>
  </body>
</html>",
    )
}

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

fn strip_frontmatter(input: &str) -> (Option<&str>, &str) {
    let mut lines = input.lines();
    if let Some(first_line) = lines.next() {
        if first_line.trim_end() != "---" {
            return (None, input);
        };

        let rest = match input.split_once('\n') {
            Some((_first_line, rest)) => rest,
            None => {
                return (None, input);
            }
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

    let (frontmatter_yaml, markdown) = strip_frontmatter(&markdown);
    let frontmatter = match frontmatter_yaml {
        Some(value) => match serde_yaml::from_str(value) {
            Ok(frontmatter_value) => frontmatter_value,
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
        let expected_result = "<!DOCTYPE html>\n<html lang=\"en\">\n  <head>\n      <meta charset=\"UTF-8\" >\n      <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" >\n      <link rel=\"icon\" href=\"data:image/x-icon;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAMAAABEpIrGAAAACXBIWXMAAAEuAAABLgF7cRpNAAAAGXRFWHRTb2Z0d2FyZQB3d3cuaW5rc2NhcGUub3Jnm+48GgAAAMlQTFRFHHaPHHaPHXePHneQH3iQIXmSInqSI3qTJXuTJ3yUK3+WLYCXL4GYMYOaM4SaNYWbN4acOYidQo2iSpKmS5KmUpeqWpyuXp6vYJ+wYaCxZqOzaKS0bae3bqe3dKu6eK28e6++hbXDhrbDh7bEiLfEirnFi7nGkLzIncTPoMbQocbQosfRp8rUrM3Wrc7XstDZs9HatdLbudXcwtrhx93jyd7k0+Xp1ubr2unt2+rt4u3x5/Dz6vL07/X38fb48/f5+/z9/v7/////WdYCwAAAAAF0Uk5T/hrjB30AAAC8SURBVDjLzdPHEoJADAZgVlAEGyr2gooNbNh7+9//ocwwjqclXsnpn53vkGR3FUXwpcQImEFYC9+tpqQgj1/dx0keAFtdDhzTzJW7Z0ojOeiEyTgC1wQDRJtigQM2xRIHasA7w4ElcGCaVIeUWnKwGgzc2YWC92eTvQQLNkbkXTimNaUJmpGAmlR3wMNigCg+gb3GANGl0OeAWAMvmwPZG3BKc6tuUPQ5IOY0a132aCvfM30SBJ4Ww58VWR+3BzKDC1fSbwAAAABJRU5ErkJggg==\" sizes=\"any\" >\n      <link rel=\"icon\" type=\"image/svg+xml\"\n      href=\"data:image/svg+xml,%3C%3Fxml version='1.0' encoding='UTF-8'%3F%3E%3Csvg width='400' height='400' version='1.1' viewBox='0 0 105.83 105.83' xmlns='http://www.w3.org/2000/svg' xmlns:cc='http://creativecommons.org/ns%23' xmlns:dc='http://purl.org/dc/elements/1.1/' xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns%23'%3E%3Cmetadata%3E%3Crdf:RDF%3E%3Ccc:Work rdf:about=''%3E%3Cdc:format%3Eimage/svg+xml%3C/dc:format%3E%3Cdc:type rdf:resource='http://purl.org/dc/dcmitype/StillImage'/%3E%3Cdc:title/%3E%3C/cc:Work%3E%3C/rdf:RDF%3E%3C/metadata%3E%3Crect x='1.7013' y='1.6799' width='102.47' height='102.47' fill='%231c768f' stroke='%231c768f' stroke-width='3.3641'/%3E%3Cg transform='matrix(2.6253 0 0 2.6253 -51.363 -97.03)' fill='%23fff' opacity='.998' style='font-variant-caps:normal;font-variant-east-asian:normal;font-variant-ligatures:normal;font-variant-numeric:normal' aria-label='R'%3E%3Cpath d='m37.305 56.556q1.4911 0 2.6094-0.35413 1.1183-0.37277 1.8638-1.0251t1.1183-1.547q0.37277-0.91328 0.37277-2.013 0-2.1993-1.4538-3.3549t-4.3987-1.1556h-3.5413v9.4497zm12.637 13.979h-3.8954q-1.1556 0-1.6775-0.89464l-6.2625-9.0396q-0.31685-0.46596-0.68962-0.67098t-1.1183-0.20502h-2.423v10.81h-4.3614v-26.839h7.9027q2.6467 0 4.5478 0.54052 1.9198 0.54052 3.1499 1.547 1.2301 0.98784 1.8079 2.3857 0.59643 1.3979 0.59643 3.1126 0 1.3979-0.42868 2.6094-0.41005 1.2115-1.2115 2.1993-0.78282 0.98784-1.9384 1.7147-1.1556 0.7269-2.628 1.1369 0.80145 0.4846 1.3792 1.3606z' fill='%23fff' style='font-variant-caps:normal;font-variant-east-asian:normal;font-variant-ligatures:normal;font-variant-numeric:normal'/%3E%3C/g%3E%3C/svg%3E\" />\n      <link rel=\"apple-touch-icon\" href=\"data:image/png;base64,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\" >\n      <meta name=\"theme-color\" content=\"#032539\" >\n      <style>:root{--max-width-full:100%;--max-width-wrapper:38rem;--spacing-px:0.0625rem;--spacing-px-2:0.125rem;--spacing-0:0;--spacing-1:0.25rem;--spacing-4:1rem;--spacing-6:1.5rem;--spacing-12:3rem;--spacing-16:4rem;--font-family:\"Helvetica Neue\", Helvetica, \"Segoe UI\", Arial, freesans,\n        sans-serif;--font-weight-normal:400;--font-weight-bold:700;--font-weight-black:900;--font-size-root:18px;--font-size-0:0.9rem;--font-size-1:1.125rem;--font-size-2:1.406rem;--font-size-4:2.197rem;--font-size-5:2.747rem;--font-size-6:3.433rem;--line-height-tight:1.3;--line-height-normal:1.5;--line-height-relaxed:1.75;--colour-heading:hsl(200 7% 8%);--color-heading-black:hsl(0 0% 0%);--colour-text:hsl(207 43% 9%)}*,:after,:before{box-sizing:border-box}*{margin:0}html{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;scroll-behavior:smooth}@media (prefers-reduced-motion:reduce){html{scroll-behavior:auto}}body{display:flex;font:1.125rem/1.5\"Helvetica Neue\",Helvetica,\"Segoe UI\",Arial,freesans,sans-serif;font:var(--font-size-1)/var(--line-height-normal) var(--font-family);color:hsl(207 43% 9%);color:var(--colour-text);text-rendering:optimizelegibility}main{max-width:38rem;max-width:var(--max-width-wrapper);margin-block:4rem;margin-block:var(--spacing-16);margin-inline:auto}h1{font-size:2.747rem;font-size:var(--font-size-5)}h2{font-size:2.197rem;font-size:var(--font-size-4)}h3{font-size:var(--font-size-3)}h4{font-size:1.406rem;font-size:var(--font-size-2)}h1,h2,h3,h4,h5,h6{margin:3rem 0 1.5rem;margin:var(--spacing-12) var(--spacing-0) var(--spacing-6);line-height:1.3;line-height:var(--line-height-tight)}h2,h3,h4,h5,h6{font-weight:700;font-weight:var(--font-weight-bold);color:hsl(200 7% 8%);color:var(--colour-heading)}p{line-height:1.75;line-height:var(--line-height-relaxed);margin:0 0 1rem;margin:var(--spacing-0) var(--spacing-0) var(--spacing-4);padding:0;padding:var(--spacing-0)}p code{background-color:#e8f1f4;background-color:var(--colour-theme-3-tint-90);border-radius:.125rem;border-radius:var(--spacing-px-2);padding:.0625rem .25rem;padding:var(--spacing-px) var(--spacing-1);-webkit-box-decoration-break:clone;box-decoration-break:clone}pre{margin-top:3rem;margin-top:var(--spacing-12);margin-bottom:4rem;margin-bottom:var(--spacing-16);width:100%;width:var(--max-width-full);max-width:100%;max-width:var(--max-width-full);overflow-x:auto}.heading-anchor{display:none}h2:hover .heading-anchor{display:inline}</style> \n<title>Test Document</title>\n\n\n  </head>\n\n  <body>\n    <main>\n      <h1 id=\"test\">Test</h1>\n<p>This is a test.</p>\n\n  </main>\n  </body>\n</html>";
        assert_eq!(html, expected_result);

        // cleanup
        remove_file(html_path).expect("Unable to delete HTML output in cleanup");
    }
}
