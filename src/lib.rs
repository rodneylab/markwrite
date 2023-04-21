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
    path: &str,
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
    display_grammar_check_results(&combined_grammar_check_results, path, stdout_handle);
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
    let prismjs_light_theme_css = include_bytes!("./resources/prism-one-light.css");
    let prismjs_style_tag = format!(
        "<style>{}</style>",
        String::from_utf8_lossy(prismjs_light_theme_css)
    );
    let prismjs_script = include_bytes!("./resources/prism.js");
    let prismjs_script_tag = format!(
        "<script>{}</script>",
        String::from_utf8_lossy(prismjs_script)
    );
    let global_styles_css = include_bytes!("./resources/styles.css");
    let global_styles_tag = format!(
        "<style>{}</style>",
        String::from_utf8_lossy(global_styles_css)
    );
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
      {global_styles_tag}
      {prismjs_style_tag}"##
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
  {prismjs_script_tag}
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

    let display_path = path.as_ref().display().to_string();
    if markwrite_options.check_grammar() {
        grammar_check(markdown, dictionary, &display_path, stdout_handle).await;
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
        let expected_result = r##"<!DOCTYPE html>
<html lang="en">
  <head>
      <meta charset="UTF-8" >
      <meta name="viewport" content="width=device-width, initial-scale=1.0" >
      <link rel="icon" href="data:image/x-icon;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAMAAABEpIrGAAAACXBIWXMAAAEuAAABLgF7cRpNAAAAGXRFWHRTb2Z0d2FyZQB3d3cuaW5rc2NhcGUub3Jnm+48GgAAAMlQTFRFHHaPHHaPHXePHneQH3iQIXmSInqSI3qTJXuTJ3yUK3+WLYCXL4GYMYOaM4SaNYWbN4acOYidQo2iSpKmS5KmUpeqWpyuXp6vYJ+wYaCxZqOzaKS0bae3bqe3dKu6eK28e6++hbXDhrbDh7bEiLfEirnFi7nGkLzIncTPoMbQocbQosfRp8rUrM3Wrc7XstDZs9HatdLbudXcwtrhx93jyd7k0+Xp1ubr2unt2+rt4u3x5/Dz6vL07/X38fb48/f5+/z9/v7/////WdYCwAAAAAF0Uk5T/hrjB30AAAC8SURBVDjLzdPHEoJADAZgVlAEGyr2gooNbNh7+9//ocwwjqclXsnpn53vkGR3FUXwpcQImEFYC9+tpqQgj1/dx0keAFtdDhzTzJW7Z0ojOeiEyTgC1wQDRJtigQM2xRIHasA7w4ElcGCaVIeUWnKwGgzc2YWC92eTvQQLNkbkXTimNaUJmpGAmlR3wMNigCg+gb3GANGl0OeAWAMvmwPZG3BKc6tuUPQ5IOY0a132aCvfM30SBJ4Ww58VWR+3BzKDC1fSbwAAAABJRU5ErkJggg==" sizes="any" >
      <link rel="icon" type="image/svg+xml"
      href="data:image/svg+xml,%3C%3Fxml version='1.0' encoding='UTF-8'%3F%3E%3Csvg width='400' height='400' version='1.1' viewBox='0 0 105.83 105.83' xmlns='http://www.w3.org/2000/svg' xmlns:cc='http://creativecommons.org/ns%23' xmlns:dc='http://purl.org/dc/elements/1.1/' xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns%23'%3E%3Cmetadata%3E%3Crdf:RDF%3E%3Ccc:Work rdf:about=''%3E%3Cdc:format%3Eimage/svg+xml%3C/dc:format%3E%3Cdc:type rdf:resource='http://purl.org/dc/dcmitype/StillImage'/%3E%3Cdc:title/%3E%3C/cc:Work%3E%3C/rdf:RDF%3E%3C/metadata%3E%3Crect x='1.7013' y='1.6799' width='102.47' height='102.47' fill='%231c768f' stroke='%231c768f' stroke-width='3.3641'/%3E%3Cg transform='matrix(2.6253 0 0 2.6253 -51.363 -97.03)' fill='%23fff' opacity='.998' style='font-variant-caps:normal;font-variant-east-asian:normal;font-variant-ligatures:normal;font-variant-numeric:normal' aria-label='R'%3E%3Cpath d='m37.305 56.556q1.4911 0 2.6094-0.35413 1.1183-0.37277 1.8638-1.0251t1.1183-1.547q0.37277-0.91328 0.37277-2.013 0-2.1993-1.4538-3.3549t-4.3987-1.1556h-3.5413v9.4497zm12.637 13.979h-3.8954q-1.1556 0-1.6775-0.89464l-6.2625-9.0396q-0.31685-0.46596-0.68962-0.67098t-1.1183-0.20502h-2.423v10.81h-4.3614v-26.839h7.9027q2.6467 0 4.5478 0.54052 1.9198 0.54052 3.1499 1.547 1.2301 0.98784 1.8079 2.3857 0.59643 1.3979 0.59643 3.1126 0 1.3979-0.42868 2.6094-0.41005 1.2115-1.2115 2.1993-0.78282 0.98784-1.9384 1.7147-1.1556 0.7269-2.628 1.1369 0.80145 0.4846 1.3792 1.3606z' fill='%23fff' style='font-variant-caps:normal;font-variant-east-asian:normal;font-variant-ligatures:normal;font-variant-numeric:normal'/%3E%3C/g%3E%3C/svg%3E" />
      <link rel="apple-touch-icon" href="data:image/png;base64,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" >
      <meta name="theme-color" content="#032539" >
      <style>:root{--max-width-full:100%;--max-width-wrapper:38rem;--spacing-px:0.0625rem;--spacing-px-2:0.125rem;--spacing-0:0;--spacing-1:0.25rem;--spacing-4:1rem;--spacing-6:1.5rem;--spacing-8:2rem;--spacing-12:3rem;--spacing-16:4rem;--font-family:"Helvetica Neue", Helvetica, "Segoe UI", Arial, freesans,
    sans-serif;--font-weight-normal:400;--font-weight-bold:700;--font-weight-black:900;--font-size-root:18px;--font-size-0:0.9rem;--font-size-1:1.125rem;--font-size-2:1.406rem;--font-size-4:2.197rem;--font-size-5:2.747rem;--font-size-6:3.433rem;--line-height-tight:1.3;--line-height-normal:1.5;--line-height-relaxed:1.75;--colour-primary:hsl(202 47% 21%);--colour-primary-tint-10:hsl(202 29% 29%);--colour-secondary:hsl(193 67% 34%);--colour-secondary-tint-90:hsl(195 35% 93%);--colour-alt:hsl(44 94% 58%);--colour-text:hsl(11 18% 12%);--colour-light:hsl(60 14% 99%)}*,:after,:before{box-sizing:border-box}*{margin:0}html{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale;scroll-behavior:smooth}@media (prefers-reduced-motion:reduce){html{scroll-behavior:auto}}body{display:flex;font:1.125rem/1.5"Helvetica Neue",Helvetica,"Segoe UI",Arial,freesans,sans-serif;font:var(--font-size-1)/var(--line-height-normal) var(--font-family);color:hsl(11 18% 12%);color:var(--colour-text);text-rendering:optimizelegibility;background-color:hsl(60 14% 99%);background-color:var(--colour-light)}main{max-width:38rem;max-width:var(--max-width-wrapper);margin-block:4rem;margin-block:var(--spacing-16);margin-inline:auto}h1,h2{font-size:2.747rem;font-size:var(--font-size-5);color:hsl(202 47% 21%);color:var(--colour-primary)}h2{font-size:2.197rem;font-size:var(--font-size-4)}h3{font-size:var(--font-size-3)}h4{font-size:1.406rem;font-size:var(--font-size-2)}h1,h2,h3,h4,h5,h6{margin:3rem 0 1.5rem;margin:var(--spacing-12) var(--spacing-0) var(--spacing-6);line-height:1.3;line-height:var(--line-height-tight)}h2,h3,h4,h5,h6{font-weight:700;font-weight:var(--font-weight-bold)}p{line-height:1.75;line-height:var(--line-height-relaxed);margin:0 0 1rem;margin:var(--spacing-0) var(--spacing-0) var(--spacing-4);padding:0;padding:var(--spacing-0)}p code{background-color:#e8f1f4;background-color:hsl(195 35% 93%);background-color:var(--colour-secondary-tint-90);border-radius:.125rem;border-radius:var(--spacing-px-2);padding:.0625rem .25rem;padding:var(--spacing-px) var(--spacing-1);-webkit-box-decoration-break:clone;box-decoration-break:clone;margin-bottom:1rem;margin-bottom:var(--spacing-4)}ol,ul{margin-inline:0;margin-inline:var(--spacing-0);margin-bottom:2rem;margin-bottom:var(--spacing-8);list-style-position:inside}:is(ol,ul) li{margin-bottom:1rem;margin-bottom:var(--spacing-4)}li>ul{margin-left:2rem;margin-left:var(--spacing-8)}li:last-child{margin-bottom:0;margin-bottom:var(--spacing-0)}a{color:hsl(202 29% 29%);color:var(--colour-primary-tint-10)}a:focus,a:hover{text-decoration:none}img,pre{max-width:100%}pre{margin-top:3rem;margin-top:var(--spacing-12);margin-bottom:4rem;margin-bottom:var(--spacing-16);width:100%;width:var(--max-width-full);max-width:var(--max-width-full);overflow-x:auto}img{margin:2rem 0 1.5rem;margin:var(--spacing-8)0 var(--spacing-6)}.heading-anchor{display:none}h2:hover .heading-anchor{display:inline}
</style>
      <style>code[class*=language-],pre[class*=language-]{background:#fafafa;color:#383a42;font-family:"Fira Code","Fira Mono",Menlo,Consolas,"DejaVu Sans Mono",monospace;direction:ltr;text-align:left;white-space:pre;word-spacing:normal;word-break:normal;line-height:1.5;-moz-tab-size:2;-o-tab-size:2;tab-size:2;-webkit-hyphens:none;hyphens:none}code[class*=language-] ::-moz-selection,code[class*=language-]::-moz-selection,pre[class*=language-] ::-moz-selection{background:#e5e5e6;color:inherit}code[class*=language-] ::selection,code[class*=language-]::selection,pre[class*=language-] ::selection{background:#e5e5e6;color:inherit}pre[class*=language-]{padding:1em;margin:.5em 0;overflow:auto;border-radius:.3em}:not(pre)>code[class*=language-]{padding:.2em .3em;border-radius:.3em;white-space:normal}.token.cdata,.token.comment,.token.prolog{color:#a0a1a7}.token.attr-value>.token.punctuation.attr-equals,.token.doctype,.token.entity,.token.punctuation,.token.special-attr>.token.attr-value>.token.value.css{color:#383a42}.token.atrule,.token.attr-name,.token.boolean,.token.class-name,.token.constant,.token.number{color:#b76b01}.token.keyword{color:#a626a4}.language-css .token.selector,.token.deleted,.token.important,.token.property,.token.symbol,.token.tag{color:#e45649}.language-css .token.url>.token.string.url,.token.attr-value,.token.attr-value>.token.punctuation,.token.builtin,.token.char,.token.inserted,.token.regex,.token.selector,.token.string{color:#50a14f}.token.function,.token.operator,.token.variable{color:#4078f2}.language-css .token.property{color:#383a42}.language-css .token.function,.language-css .token.url>.token.function,.token.url{color:#0184bc}.language-css .token.atrule .token.rule,.language-css .token.important,.language-javascript .token.operator{color:#a626a4}.language-javascript .token.template-string>.token.interpolation>.token.interpolation-punctuation.punctuation{color:#ca1243}.language-json .token.operator,.language-markdown .token.url,.language-markdown .token.url-reference.url>.token.string,.language-markdown .token.url>.token.operator{color:#383a42}.language-json .token.null.keyword{color:#b76b01}.language-markdown .token.url>.token.content{color:#4078f2}.language-markdown .token.url-reference.url,.language-markdown .token.url>.token.url{color:#0184bc}.language-markdown .token.blockquote.punctuation,.language-markdown .token.hr.punctuation{color:#a0a1a7;font-style:italic}.language-markdown .token.code-snippet{color:#50a14f}.language-markdown .token.bold .token.content{color:#b76b01}.language-markdown .token.italic .token.content{color:#a626a4}.language-markdown .token.list.punctuation,.language-markdown .token.strike .token.content,.language-markdown .token.strike .token.punctuation,.language-markdown .token.title.important>.token.punctuation,.rainbow-braces .token.token.punctuation.brace-level-1,.rainbow-braces .token.token.punctuation.brace-level-5,.rainbow-braces .token.token.punctuation.brace-level-9{color:#e45649}.token.bold{font-weight:700}.token.comment,.token.italic{font-style:italic}.token.entity{cursor:help}.token.namespace{opacity:.8}.token.token.cr:before,.token.token.lf:before,.token.token.space:before,.token.token.tab:not(:empty):before{color:rgba(56,58,66,.2)}div.code-toolbar>.toolbar.toolbar>.toolbar-item{margin-right:.4em}div.code-toolbar>.toolbar.toolbar>.toolbar-item>a,div.code-toolbar>.toolbar.toolbar>.toolbar-item>button,div.code-toolbar>.toolbar.toolbar>.toolbar-item>span{background:#e5e5e6;color:#696c77;padding:.1em .4em;border-radius:.3em}div.code-toolbar>.toolbar.toolbar>.toolbar-item>a:focus,div.code-toolbar>.toolbar.toolbar>.toolbar-item>a:hover,div.code-toolbar>.toolbar.toolbar>.toolbar-item>button:focus,div.code-toolbar>.toolbar.toolbar>.toolbar-item>button:hover,div.code-toolbar>.toolbar.toolbar>.toolbar-item>span:focus,div.code-toolbar>.toolbar.toolbar>.toolbar-item>span:hover{background:#c6c7c7;color:#383a42}.line-highlight.line-highlight{background:rgba(56,58,66,.05)}.line-highlight.line-highlight:before,.line-highlight.line-highlight[data-end]:after{background:#e5e5e6;color:#383a42;padding:.1em .6em;border-radius:.3em;box-shadow:0 2px 0 0 rgba(0,0,0,.2)}pre[id].linkable-line-numbers.linkable-line-numbers span.line-numbers-rows>span:hover:before{background-color:rgba(56,58,66,.05)}.command-line .command-line-prompt,.line-numbers.line-numbers .line-numbers-rows{border-right-color:rgba(56,58,66,.2)}.command-line .command-line-prompt>span:before,.line-numbers .line-numbers-rows>span:before{color:#9d9d9f}.rainbow-braces .token.token.punctuation.brace-level-10,.rainbow-braces .token.token.punctuation.brace-level-2,.rainbow-braces .token.token.punctuation.brace-level-6{color:#50a14f}.rainbow-braces .token.token.punctuation.brace-level-11,.rainbow-braces .token.token.punctuation.brace-level-3,.rainbow-braces .token.token.punctuation.brace-level-7{color:#4078f2}.rainbow-braces .token.token.punctuation.brace-level-12,.rainbow-braces .token.token.punctuation.brace-level-4,.rainbow-braces .token.token.punctuation.brace-level-8{color:#a626a4}pre.diff-highlight>code .token.token.deleted:not(.prefix),pre>code.diff-highlight .token.token.deleted:not(.prefix){background-color:rgba(255,82,102,.15)}pre.diff-highlight>code .token.token.deleted:not(.prefix) ::-moz-selection,pre.diff-highlight>code .token.token.deleted:not(.prefix)::-moz-selection,pre>code.diff-highlight .token.token.deleted:not(.prefix) ::-moz-selection,pre>code.diff-highlight .token.token.deleted:not(.prefix)::-moz-selection{background-color:rgba(251,86,105,.25)}pre.diff-highlight>code .token.token.deleted:not(.prefix) ::selection,pre.diff-highlight>code .token.token.deleted:not(.prefix)::selection,pre>code.diff-highlight .token.token.deleted:not(.prefix) ::selection,pre>code.diff-highlight .token.token.deleted:not(.prefix)::selection{background-color:rgba(251,86,105,.25)}pre.diff-highlight>code .token.token.inserted:not(.prefix),pre>code.diff-highlight .token.token.inserted:not(.prefix){background-color:rgba(26,255,91,.15)}pre.diff-highlight>code .token.token.inserted:not(.prefix) ::-moz-selection,pre.diff-highlight>code .token.token.inserted:not(.prefix)::-moz-selection,pre>code.diff-highlight .token.token.inserted:not(.prefix) ::-moz-selection,pre>code.diff-highlight .token.token.inserted:not(.prefix)::-moz-selection{background-color:rgba(56,224,98,.25)}pre.diff-highlight>code .token.token.inserted:not(.prefix) ::selection,pre.diff-highlight>code .token.token.inserted:not(.prefix)::selection,pre>code.diff-highlight .token.token.inserted:not(.prefix) ::selection,pre>code.diff-highlight .token.token.inserted:not(.prefix)::selection{background-color:rgba(56,224,98,.25)}.prism-previewer-gradient.prism-previewer-gradient div,.prism-previewer.prism-previewer:before{border-color:hsl(0,0,95%)}.prism-previewer-color.prism-previewer-color:before,.prism-previewer-easing.prism-previewer-easing:before,.prism-previewer-gradient.prism-previewer-gradient div{border-radius:.3em}.prism-previewer.prism-previewer:after{border-top-color:hsl(0,0,95%)}.prism-previewer-flipped.prism-previewer-flipped.after{border-bottom-color:hsl(0,0,95%)}.prism-previewer-angle.prism-previewer-angle:before,.prism-previewer-easing.prism-previewer-easing,.prism-previewer-time.prism-previewer-time:before{background:#fff}.prism-previewer-angle.prism-previewer-angle circle,.prism-previewer-time.prism-previewer-time circle{stroke:#383a42;stroke-opacity:1}.prism-previewer-easing.prism-previewer-easing circle,.prism-previewer-easing.prism-previewer-easing line,.prism-previewer-easing.prism-previewer-easing path{stroke:#383a42}.prism-previewer-easing.prism-previewer-easing circle{fill:transparent}</style>
<title>Test Document</title>


  </head>

  <body>
    <main>
      <h1 id="test">Test</h1>
<p>This is a test.</p>

  </main>
  <script>/* PrismJS 1.29.0
https://prismjs.com/download.html#themes=prism&languages=markup+css+clike+javascript+markdown+typescript */
var _self="undefined"!=typeof window?window:"undefined"!=typeof WorkerGlobalScope&&self instanceof WorkerGlobalScope?self:{},Prism=function(e){var n=/(?:^|\s)lang(?:uage)?-([\w-]+)(?=\s|$)/i,t=0,r={},a={manual:e.Prism&&e.Prism.manual,disableWorkerMessageHandler:e.Prism&&e.Prism.disableWorkerMessageHandler,util:{encode:function e(n){return n instanceof i?new i(n.type,e(n.content),n.alias):Array.isArray(n)?n.map(e):n.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/\u00a0/g," ")},type:function(e){return Object.prototype.toString.call(e).slice(8,-1)},objId:function(e){return e.__id||Object.defineProperty(e,"__id",{value:++t}),e.__id},clone:function e(n,t){var r,i;switch(t=t||{},a.util.type(n)){case"Object":if(i=a.util.objId(n),t[i])return t[i];for(var l in r={},t[i]=r,n)n.hasOwnProperty(l)&&(r[l]=e(n[l],t));return r;case"Array":return i=a.util.objId(n),t[i]?t[i]:(r=[],t[i]=r,n.forEach((function(n,a){r[a]=e(n,t)})),r);default:return n}},getLanguage:function(e){for(;e;){var t=n.exec(e.className);if(t)return t[1].toLowerCase();e=e.parentElement}return"none"},setLanguage:function(e,t){e.className=e.className.replace(RegExp(n,"gi"),""),e.classList.add("language-"+t)},currentScript:function(){if("undefined"==typeof document)return null;if("currentScript"in document)return document.currentScript;try{throw new Error}catch(r){var e=(/at [^(\r\n]*\((.*):[^:]+:[^:]+\)$/i.exec(r.stack)||[])[1];if(e){var n=document.getElementsByTagName("script");for(var t in n)if(n[t].src==e)return n[t]}return null}},isActive:function(e,n,t){for(var r="no-"+n;e;){var a=e.classList;if(a.contains(n))return!0;if(a.contains(r))return!1;e=e.parentElement}return!!t}},languages:{plain:r,plaintext:r,text:r,txt:r,extend:function(e,n){var t=a.util.clone(a.languages[e]);for(var r in n)t[r]=n[r];return t},insertBefore:function(e,n,t,r){var i=(r=r||a.languages)[e],l={};for(var o in i)if(i.hasOwnProperty(o)){if(o==n)for(var s in t)t.hasOwnProperty(s)&&(l[s]=t[s]);t.hasOwnProperty(o)||(l[o]=i[o])}var u=r[e];return r[e]=l,a.languages.DFS(a.languages,(function(n,t){t===u&&n!=e&&(this[n]=l)})),l},DFS:function e(n,t,r,i){i=i||{};var l=a.util.objId;for(var o in n)if(n.hasOwnProperty(o)){t.call(n,o,n[o],r||o);var s=n[o],u=a.util.type(s);"Object"!==u||i[l(s)]?"Array"!==u||i[l(s)]||(i[l(s)]=!0,e(s,t,o,i)):(i[l(s)]=!0,e(s,t,null,i))}}},plugins:{},highlightAll:function(e,n){a.highlightAllUnder(document,e,n)},highlightAllUnder:function(e,n,t){var r={callback:t,container:e,selector:'code[class*="language-"], [class*="language-"] code, code[class*="lang-"], [class*="lang-"] code'};a.hooks.run("before-highlightall",r),r.elements=Array.prototype.slice.apply(r.container.querySelectorAll(r.selector)),a.hooks.run("before-all-elements-highlight",r);for(var i,l=0;i=r.elements[l++];)a.highlightElement(i,!0===n,r.callback)},highlightElement:function(n,t,r){var i=a.util.getLanguage(n),l=a.languages[i];a.util.setLanguage(n,i);var o=n.parentElement;o&&"pre"===o.nodeName.toLowerCase()&&a.util.setLanguage(o,i);var s={element:n,language:i,grammar:l,code:n.textContent};function u(e){s.highlightedCode=e,a.hooks.run("before-insert",s),s.element.innerHTML=s.highlightedCode,a.hooks.run("after-highlight",s),a.hooks.run("complete",s),r&&r.call(s.element)}if(a.hooks.run("before-sanity-check",s),(o=s.element.parentElement)&&"pre"===o.nodeName.toLowerCase()&&!o.hasAttribute("tabindex")&&o.setAttribute("tabindex","0"),!s.code)return a.hooks.run("complete",s),void(r&&r.call(s.element));if(a.hooks.run("before-highlight",s),s.grammar)if(t&&e.Worker){var c=new Worker(a.filename);c.onmessage=function(e){u(e.data)},c.postMessage(JSON.stringify({language:s.language,code:s.code,immediateClose:!0}))}else u(a.highlight(s.code,s.grammar,s.language));else u(a.util.encode(s.code))},highlight:function(e,n,t){var r={code:e,grammar:n,language:t};if(a.hooks.run("before-tokenize",r),!r.grammar)throw new Error('The language "'+r.language+'" has no grammar.');return r.tokens=a.tokenize(r.code,r.grammar),a.hooks.run("after-tokenize",r),i.stringify(a.util.encode(r.tokens),r.language)},tokenize:function(e,n){var t=n.rest;if(t){for(var r in t)n[r]=t[r];delete n.rest}var a=new s;return u(a,a.head,e),o(e,a,n,a.head,0),function(e){for(var n=[],t=e.head.next;t!==e.tail;)n.push(t.value),t=t.next;return n}(a)},hooks:{all:{},add:function(e,n){var t=a.hooks.all;t[e]=t[e]||[],t[e].push(n)},run:function(e,n){var t=a.hooks.all[e];if(t&&t.length)for(var r,i=0;r=t[i++];)r(n)}},Token:i};function i(e,n,t,r){this.type=e,this.content=n,this.alias=t,this.length=0|(r||"").length}function l(e,n,t,r){e.lastIndex=n;var a=e.exec(t);if(a&&r&&a[1]){var i=a[1].length;a.index+=i,a[0]=a[0].slice(i)}return a}function o(e,n,t,r,s,g){for(var f in t)if(t.hasOwnProperty(f)&&t[f]){var h=t[f];h=Array.isArray(h)?h:[h];for(var d=0;d<h.length;++d){if(g&&g.cause==f+","+d)return;var v=h[d],p=v.inside,m=!!v.lookbehind,y=!!v.greedy,k=v.alias;if(y&&!v.pattern.global){var x=v.pattern.toString().match(/[imsuy]*$/)[0];v.pattern=RegExp(v.pattern.source,x+"g")}for(var b=v.pattern||v,w=r.next,A=s;w!==n.tail&&!(g&&A>=g.reach);A+=w.value.length,w=w.next){var E=w.value;if(n.length>e.length)return;if(!(E instanceof i)){var P,L=1;if(y){if(!(P=l(b,A,e,m))||P.index>=e.length)break;var S=P.index,O=P.index+P[0].length,j=A;for(j+=w.value.length;S>=j;)j+=(w=w.next).value.length;if(A=j-=w.value.length,w.value instanceof i)continue;for(var C=w;C!==n.tail&&(j<O||"string"==typeof C.value);C=C.next)L++,j+=C.value.length;L--,E=e.slice(A,j),P.index-=A}else if(!(P=l(b,0,E,m)))continue;S=P.index;var N=P[0],_=E.slice(0,S),M=E.slice(S+N.length),W=A+E.length;g&&W>g.reach&&(g.reach=W);var z=w.prev;if(_&&(z=u(n,z,_),A+=_.length),c(n,z,L),w=u(n,z,new i(f,p?a.tokenize(N,p):N,k,N)),M&&u(n,w,M),L>1){var I={cause:f+","+d,reach:W};o(e,n,t,w.prev,A,I),g&&I.reach>g.reach&&(g.reach=I.reach)}}}}}}function s(){var e={value:null,prev:null,next:null},n={value:null,prev:e,next:null};e.next=n,this.head=e,this.tail=n,this.length=0}function u(e,n,t){var r=n.next,a={value:t,prev:n,next:r};return n.next=a,r.prev=a,e.length++,a}function c(e,n,t){for(var r=n.next,a=0;a<t&&r!==e.tail;a++)r=r.next;n.next=r,r.prev=n,e.length-=a}if(e.Prism=a,i.stringify=function e(n,t){if("string"==typeof n)return n;if(Array.isArray(n)){var r="";return n.forEach((function(n){r+=e(n,t)})),r}var i={type:n.type,content:e(n.content,t),tag:"span",classes:["token",n.type],attributes:{},language:t},l=n.alias;l&&(Array.isArray(l)?Array.prototype.push.apply(i.classes,l):i.classes.push(l)),a.hooks.run("wrap",i);var o="";for(var s in i.attributes)o+=" "+s+'="'+(i.attributes[s]||"").replace(/"/g,"&quot;")+'"';return"<"+i.tag+' class="'+i.classes.join(" ")+'"'+o+">"+i.content+"</"+i.tag+">"},!e.document)return e.addEventListener?(a.disableWorkerMessageHandler||e.addEventListener("message",(function(n){var t=JSON.parse(n.data),r=t.language,i=t.code,l=t.immediateClose;e.postMessage(a.highlight(i,a.languages[r],r)),l&&e.close()}),!1),a):a;var g=a.util.currentScript();function f(){a.manual||a.highlightAll()}if(g&&(a.filename=g.src,g.hasAttribute("data-manual")&&(a.manual=!0)),!a.manual){var h=document.readyState;"loading"===h||"interactive"===h&&g&&g.defer?document.addEventListener("DOMContentLoaded",f):window.requestAnimationFrame?window.requestAnimationFrame(f):window.setTimeout(f,16)}return a}(_self);"undefined"!=typeof module&&module.exports&&(module.exports=Prism),"undefined"!=typeof global&&(global.Prism=Prism);
Prism.languages.markup={comment:{pattern:/<!--(?:(?!<!--)[\s\S])*?-->/,greedy:!0},prolog:{pattern:/<\?[\s\S]+?\?>/,greedy:!0},doctype:{pattern:/<!DOCTYPE(?:[^>"'[\]]|"[^"]*"|'[^']*')+(?:\[(?:[^<"'\]]|"[^"]*"|'[^']*'|<(?!!--)|<!--(?:[^-]|-(?!->))*-->)*\]\s*)?>/i,greedy:!0,inside:{"internal-subset":{pattern:/(^[^\[]*\[)[\s\S]+(?=\]>$)/,lookbehind:!0,greedy:!0,inside:null},string:{pattern:/"[^"]*"|'[^']*'/,greedy:!0},punctuation:/^<!|>$|[[\]]/,"doctype-tag":/^DOCTYPE/i,name:/[^\s<>'"]+/}},cdata:{pattern:/<!\[CDATA\[[\s\S]*?\]\]>/i,greedy:!0},tag:{pattern:/<\/?(?!\d)[^\s>\/=$<%]+(?:\s(?:\s*[^\s>\/=]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s'">=]+(?=[\s>]))|(?=[\s/>])))+)?\s*\/?>/,greedy:!0,inside:{tag:{pattern:/^<\/?[^\s>\/]+/,inside:{punctuation:/^<\/?/,namespace:/^[^\s>\/:]+:/}},"special-attr":[],"attr-value":{pattern:/=\s*(?:"[^"]*"|'[^']*'|[^\s'">=]+)/,inside:{punctuation:[{pattern:/^=/,alias:"attr-equals"},{pattern:/^(\s*)["']|["']$/,lookbehind:!0}]}},punctuation:/\/?>/,"attr-name":{pattern:/[^\s>\/]+/,inside:{namespace:/^[^\s>\/:]+:/}}}},entity:[{pattern:/&[\da-z]{1,8};/i,alias:"named-entity"},/&#x?[\da-f]{1,8};/i]},Prism.languages.markup.tag.inside["attr-value"].inside.entity=Prism.languages.markup.entity,Prism.languages.markup.doctype.inside["internal-subset"].inside=Prism.languages.markup,Prism.hooks.add("wrap",(function(a){"entity"===a.type&&(a.attributes.title=a.content.replace(/&amp;/,"&"))})),Object.defineProperty(Prism.languages.markup.tag,"addInlined",{value:function(a,e){var s={};s["language-"+e]={pattern:/(^<!\[CDATA\[)[\s\S]+?(?=\]\]>$)/i,lookbehind:!0,inside:Prism.languages[e]},s.cdata=/^<!\[CDATA\[|\]\]>$/i;var t={"included-cdata":{pattern:/<!\[CDATA\[[\s\S]*?\]\]>/i,inside:s}};t["language-"+e]={pattern:/[\s\S]+/,inside:Prism.languages[e]};var n={};n[a]={pattern:RegExp("(<__[^>]*>)(?:<!\\[CDATA\\[(?:[^\\]]|\\](?!\\]>))*\\]\\]>|(?!<!\\[CDATA\\[)[^])*?(?=</__>)".replace(/__/g,(function(){return a})),"i"),lookbehind:!0,greedy:!0,inside:t},Prism.languages.insertBefore("markup","cdata",n)}}),Object.defineProperty(Prism.languages.markup.tag,"addAttribute",{value:function(a,e){Prism.languages.markup.tag.inside["special-attr"].push({pattern:RegExp("(^|[\"'\\s])(?:"+a+")\\s*=\\s*(?:\"[^\"]*\"|'[^']*'|[^\\s'\">=]+(?=[\\s>]))","i"),lookbehind:!0,inside:{"attr-name":/^[^\s=]+/,"attr-value":{pattern:/=[\s\S]+/,inside:{value:{pattern:/(^=\s*(["']|(?!["'])))\S[\s\S]*(?=\2$)/,lookbehind:!0,alias:[e,"language-"+e],inside:Prism.languages[e]},punctuation:[{pattern:/^=/,alias:"attr-equals"},/"|'/]}}}})}}),Prism.languages.html=Prism.languages.markup,Prism.languages.mathml=Prism.languages.markup,Prism.languages.svg=Prism.languages.markup,Prism.languages.xml=Prism.languages.extend("markup",{}),Prism.languages.ssml=Prism.languages.xml,Prism.languages.atom=Prism.languages.xml,Prism.languages.rss=Prism.languages.xml;
!function(s){var e=/(?:"(?:\\(?:\r\n|[\s\S])|[^"\\\r\n])*"|'(?:\\(?:\r\n|[\s\S])|[^'\\\r\n])*')/;s.languages.css={comment:/\/\*[\s\S]*?\*\//,atrule:{pattern:RegExp("@[\\w-](?:[^;{\\s\"']|\\s+(?!\\s)|"+e.source+")*?(?:;|(?=\\s*\\{))"),inside:{rule:/^@[\w-]+/,"selector-function-argument":{pattern:/(\bselector\s*\(\s*(?![\s)]))(?:[^()\s]|\s+(?![\s)])|\((?:[^()]|\([^()]*\))*\))+(?=\s*\))/,lookbehind:!0,alias:"selector"},keyword:{pattern:/(^|[^\w-])(?:and|not|only|or)(?![\w-])/,lookbehind:!0}}},url:{pattern:RegExp("\\burl\\((?:"+e.source+"|(?:[^\\\\\r\n()\"']|\\\\[^])*)\\)","i"),greedy:!0,inside:{function:/^url/i,punctuation:/^\(|\)$/,string:{pattern:RegExp("^"+e.source+"$"),alias:"url"}}},selector:{pattern:RegExp("(^|[{}\\s])[^{}\\s](?:[^{};\"'\\s]|\\s+(?![\\s{])|"+e.source+")*(?=\\s*\\{)"),lookbehind:!0},string:{pattern:e,greedy:!0},property:{pattern:/(^|[^-\w\xA0-\uFFFF])(?!\s)[-_a-z\xA0-\uFFFF](?:(?!\s)[-\w\xA0-\uFFFF])*(?=\s*:)/i,lookbehind:!0},important:/!important\b/i,function:{pattern:/(^|[^-a-z0-9])[-a-z0-9]+(?=\()/i,lookbehind:!0},punctuation:/[(){};:,]/},s.languages.css.atrule.inside.rest=s.languages.css;var t=s.languages.markup;t&&(t.tag.addInlined("style","css"),t.tag.addAttribute("style","css"))}(Prism);
Prism.languages.clike={comment:[{pattern:/(^|[^\\])\/\*[\s\S]*?(?:\*\/|$)/,lookbehind:!0,greedy:!0},{pattern:/(^|[^\\:])\/\/.*/,lookbehind:!0,greedy:!0}],string:{pattern:/(["'])(?:\\(?:\r\n|[\s\S])|(?!\1)[^\\\r\n])*\1/,greedy:!0},"class-name":{pattern:/(\b(?:class|extends|implements|instanceof|interface|new|trait)\s+|\bcatch\s+\()[\w.\\]+/i,lookbehind:!0,inside:{punctuation:/[.\\]/}},keyword:/\b(?:break|catch|continue|do|else|finally|for|function|if|in|instanceof|new|null|return|throw|try|while)\b/,boolean:/\b(?:false|true)\b/,function:/\b\w+(?=\()/,number:/\b0x[\da-f]+\b|(?:\b\d+(?:\.\d*)?|\B\.\d+)(?:e[+-]?\d+)?/i,operator:/[<>]=?|[!=]=?=?|--?|\+\+?|&&?|\|\|?|[?*/~^%]/,punctuation:/[{}[\];(),.:]/};
Prism.languages.javascript=Prism.languages.extend("clike",{"class-name":[Prism.languages.clike["class-name"],{pattern:/(^|[^$\w\xA0-\uFFFF])(?!\s)[_$A-Z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*(?=\.(?:constructor|prototype))/,lookbehind:!0}],keyword:[{pattern:/((?:^|\})\s*)catch\b/,lookbehind:!0},{pattern:/(^|[^.]|\.\.\.\s*)\b(?:as|assert(?=\s*\{)|async(?=\s*(?:function\b|\(|[$\w\xA0-\uFFFF]|$))|await|break|case|class|const|continue|debugger|default|delete|do|else|enum|export|extends|finally(?=\s*(?:\{|$))|for|from(?=\s*(?:['"]|$))|function|(?:get|set)(?=\s*(?:[#\[$\w\xA0-\uFFFF]|$))|if|implements|import|in|instanceof|interface|let|new|null|of|package|private|protected|public|return|static|super|switch|this|throw|try|typeof|undefined|var|void|while|with|yield)\b/,lookbehind:!0}],function:/#?(?!\s)[_$a-zA-Z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*(?=\s*(?:\.\s*(?:apply|bind|call)\s*)?\()/,number:{pattern:RegExp("(^|[^\\w$])(?:NaN|Infinity|0[bB][01]+(?:_[01]+)*n?|0[oO][0-7]+(?:_[0-7]+)*n?|0[xX][\\dA-Fa-f]+(?:_[\\dA-Fa-f]+)*n?|\\d+(?:_\\d+)*n|(?:\\d+(?:_\\d+)*(?:\\.(?:\\d+(?:_\\d+)*)?)?|\\.\\d+(?:_\\d+)*)(?:[Ee][+-]?\\d+(?:_\\d+)*)?)(?![\\w$])"),lookbehind:!0},operator:/--|\+\+|\*\*=?|=>|&&=?|\|\|=?|[!=]==|<<=?|>>>?=?|[-+*/%&|^!=<>]=?|\.{3}|\?\?=?|\?\.?|[~:]/}),Prism.languages.javascript["class-name"][0].pattern=/(\b(?:class|extends|implements|instanceof|interface|new)\s+)[\w.\\]+/,Prism.languages.insertBefore("javascript","keyword",{regex:{pattern:RegExp("((?:^|[^$\\w\\xA0-\\uFFFF.\"'\\])\\s]|\\b(?:return|yield))\\s*)/(?:(?:\\[(?:[^\\]\\\\\r\n]|\\\\.)*\\]|\\\\.|[^/\\\\\\[\r\n])+/[dgimyus]{0,7}|(?:\\[(?:[^[\\]\\\\\r\n]|\\\\.|\\[(?:[^[\\]\\\\\r\n]|\\\\.|\\[(?:[^[\\]\\\\\r\n]|\\\\.)*\\])*\\])*\\]|\\\\.|[^/\\\\\\[\r\n])+/[dgimyus]{0,7}v[dgimyus]{0,7})(?=(?:\\s|/\\*(?:[^*]|\\*(?!/))*\\*/)*(?:$|[\r\n,.;:})\\]]|//))"),lookbehind:!0,greedy:!0,inside:{"regex-source":{pattern:/^(\/)[\s\S]+(?=\/[a-z]*$)/,lookbehind:!0,alias:"language-regex",inside:Prism.languages.regex},"regex-delimiter":/^\/|\/$/,"regex-flags":/^[a-z]+$/}},"function-variable":{pattern:/#?(?!\s)[_$a-zA-Z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*(?=\s*[=:]\s*(?:async\s*)?(?:\bfunction\b|(?:\((?:[^()]|\([^()]*\))*\)|(?!\s)[_$a-zA-Z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*)\s*=>))/,alias:"function"},parameter:[{pattern:/(function(?:\s+(?!\s)[_$a-zA-Z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*)?\s*\(\s*)(?!\s)(?:[^()\s]|\s+(?![\s)])|\([^()]*\))+(?=\s*\))/,lookbehind:!0,inside:Prism.languages.javascript},{pattern:/(^|[^$\w\xA0-\uFFFF])(?!\s)[_$a-z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*(?=\s*=>)/i,lookbehind:!0,inside:Prism.languages.javascript},{pattern:/(\(\s*)(?!\s)(?:[^()\s]|\s+(?![\s)])|\([^()]*\))+(?=\s*\)\s*=>)/,lookbehind:!0,inside:Prism.languages.javascript},{pattern:/((?:\b|\s|^)(?!(?:as|async|await|break|case|catch|class|const|continue|debugger|default|delete|do|else|enum|export|extends|finally|for|from|function|get|if|implements|import|in|instanceof|interface|let|new|null|of|package|private|protected|public|return|set|static|super|switch|this|throw|try|typeof|undefined|var|void|while|with|yield)(?![$\w\xA0-\uFFFF]))(?:(?!\s)[_$a-zA-Z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*\s*)\(\s*|\]\s*\(\s*)(?!\s)(?:[^()\s]|\s+(?![\s)])|\([^()]*\))+(?=\s*\)\s*\{)/,lookbehind:!0,inside:Prism.languages.javascript}],constant:/\b[A-Z](?:[A-Z_]|\dx?)*\b/}),Prism.languages.insertBefore("javascript","string",{hashbang:{pattern:/^#!.*/,greedy:!0,alias:"comment"},"template-string":{pattern:/`(?:\\[\s\S]|\$\{(?:[^{}]|\{(?:[^{}]|\{[^}]*\})*\})+\}|(?!\$\{)[^\\`])*`/,greedy:!0,inside:{"template-punctuation":{pattern:/^`|`$/,alias:"string"},interpolation:{pattern:/((?:^|[^\\])(?:\\{2})*)\$\{(?:[^{}]|\{(?:[^{}]|\{[^}]*\})*\})+\}/,lookbehind:!0,inside:{"interpolation-punctuation":{pattern:/^\$\{|\}$/,alias:"punctuation"},rest:Prism.languages.javascript}},string:/[\s\S]+/}},"string-property":{pattern:/((?:^|[,{])[ \t]*)(["'])(?:\\(?:\r\n|[\s\S])|(?!\2)[^\\\r\n])*\2(?=\s*:)/m,lookbehind:!0,greedy:!0,alias:"property"}}),Prism.languages.insertBefore("javascript","operator",{"literal-property":{pattern:/((?:^|[,{])[ \t]*)(?!\s)[_$a-zA-Z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*(?=\s*:)/m,lookbehind:!0,alias:"property"}}),Prism.languages.markup&&(Prism.languages.markup.tag.addInlined("script","javascript"),Prism.languages.markup.tag.addAttribute("on(?:abort|blur|change|click|composition(?:end|start|update)|dblclick|error|focus(?:in|out)?|key(?:down|up)|load|mouse(?:down|enter|leave|move|out|over|up)|reset|resize|scroll|select|slotchange|submit|unload|wheel)","javascript")),Prism.languages.js=Prism.languages.javascript;
!function(n){function e(n){return n=n.replace(/<inner>/g,(function(){return"(?:\\\\.|[^\\\\\n\r]|(?:\n|\r\n?)(?![\r\n]))"})),RegExp("((?:^|[^\\\\])(?:\\\\{2})*)(?:"+n+")")}var t="(?:\\\\.|``(?:[^`\r\n]|`(?!`))+``|`[^`\r\n]+`|[^\\\\|\r\n`])+",a="\\|?__(?:\\|__)+\\|?(?:(?:\n|\r\n?)|(?![^]))".replace(/__/g,(function(){return t})),i="\\|?[ \t]*:?-{3,}:?[ \t]*(?:\\|[ \t]*:?-{3,}:?[ \t]*)+\\|?(?:\n|\r\n?)";n.languages.markdown=n.languages.extend("markup",{}),n.languages.insertBefore("markdown","prolog",{"front-matter-block":{pattern:/(^(?:\s*[\r\n])?)---(?!.)[\s\S]*?[\r\n]---(?!.)/,lookbehind:!0,greedy:!0,inside:{punctuation:/^---|---$/,"front-matter":{pattern:/\S+(?:\s+\S+)*/,alias:["yaml","language-yaml"],inside:n.languages.yaml}}},blockquote:{pattern:/^>(?:[\t ]*>)*/m,alias:"punctuation"},table:{pattern:RegExp("^"+a+i+"(?:"+a+")*","m"),inside:{"table-data-rows":{pattern:RegExp("^("+a+i+")(?:"+a+")*$"),lookbehind:!0,inside:{"table-data":{pattern:RegExp(t),inside:n.languages.markdown},punctuation:/\|/}},"table-line":{pattern:RegExp("^("+a+")"+i+"$"),lookbehind:!0,inside:{punctuation:/\||:?-{3,}:?/}},"table-header-row":{pattern:RegExp("^"+a+"$"),inside:{"table-header":{pattern:RegExp(t),alias:"important",inside:n.languages.markdown},punctuation:/\|/}}}},code:[{pattern:/((?:^|\n)[ \t]*\n|(?:^|\r\n?)[ \t]*\r\n?)(?: {4}|\t).+(?:(?:\n|\r\n?)(?: {4}|\t).+)*/,lookbehind:!0,alias:"keyword"},{pattern:/^```[\s\S]*?^```$/m,greedy:!0,inside:{"code-block":{pattern:/^(```.*(?:\n|\r\n?))[\s\S]+?(?=(?:\n|\r\n?)^```$)/m,lookbehind:!0},"code-language":{pattern:/^(```).+/,lookbehind:!0},punctuation:/```/}}],title:[{pattern:/\S.*(?:\n|\r\n?)(?:==+|--+)(?=[ \t]*$)/m,alias:"important",inside:{punctuation:/==+$|--+$/}},{pattern:/(^\s*)#.+/m,lookbehind:!0,alias:"important",inside:{punctuation:/^#+|#+$/}}],hr:{pattern:/(^\s*)([*-])(?:[\t ]*\2){2,}(?=\s*$)/m,lookbehind:!0,alias:"punctuation"},list:{pattern:/(^\s*)(?:[*+-]|\d+\.)(?=[\t ].)/m,lookbehind:!0,alias:"punctuation"},"url-reference":{pattern:/!?\[[^\]]+\]:[\t ]+(?:\S+|<(?:\\.|[^>\\])+>)(?:[\t ]+(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\((?:\\.|[^)\\])*\)))?/,inside:{variable:{pattern:/^(!?\[)[^\]]+/,lookbehind:!0},string:/(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\((?:\\.|[^)\\])*\))$/,punctuation:/^[\[\]!:]|[<>]/},alias:"url"},bold:{pattern:e("\\b__(?:(?!_)<inner>|_(?:(?!_)<inner>)+_)+__\\b|\\*\\*(?:(?!\\*)<inner>|\\*(?:(?!\\*)<inner>)+\\*)+\\*\\*"),lookbehind:!0,greedy:!0,inside:{content:{pattern:/(^..)[\s\S]+(?=..$)/,lookbehind:!0,inside:{}},punctuation:/\*\*|__/}},italic:{pattern:e("\\b_(?:(?!_)<inner>|__(?:(?!_)<inner>)+__)+_\\b|\\*(?:(?!\\*)<inner>|\\*\\*(?:(?!\\*)<inner>)+\\*\\*)+\\*"),lookbehind:!0,greedy:!0,inside:{content:{pattern:/(^.)[\s\S]+(?=.$)/,lookbehind:!0,inside:{}},punctuation:/[*_]/}},strike:{pattern:e("(~~?)(?:(?!~)<inner>)+\\2"),lookbehind:!0,greedy:!0,inside:{content:{pattern:/(^~~?)[\s\S]+(?=\1$)/,lookbehind:!0,inside:{}},punctuation:/~~?/}},"code-snippet":{pattern:/(^|[^\\`])(?:``[^`\r\n]+(?:`[^`\r\n]+)*``(?!`)|`[^`\r\n]+`(?!`))/,lookbehind:!0,greedy:!0,alias:["code","keyword"]},url:{pattern:e('!?\\[(?:(?!\\])<inner>)+\\](?:\\([^\\s)]+(?:[\t ]+"(?:\\\\.|[^"\\\\])*")?\\)|[ \t]?\\[(?:(?!\\])<inner>)+\\])'),lookbehind:!0,greedy:!0,inside:{operator:/^!/,content:{pattern:/(^\[)[^\]]+(?=\])/,lookbehind:!0,inside:{}},variable:{pattern:/(^\][ \t]?\[)[^\]]+(?=\]$)/,lookbehind:!0},url:{pattern:/(^\]\()[^\s)]+/,lookbehind:!0},string:{pattern:/(^[ \t]+)"(?:\\.|[^"\\])*"(?=\)$)/,lookbehind:!0}}}}),["url","bold","italic","strike"].forEach((function(e){["url","bold","italic","strike","code-snippet"].forEach((function(t){e!==t&&(n.languages.markdown[e].inside.content.inside[t]=n.languages.markdown[t])}))})),n.hooks.add("after-tokenize",(function(n){"markdown"!==n.language&&"md"!==n.language||function n(e){if(e&&"string"!=typeof e)for(var t=0,a=e.length;t<a;t++){var i=e[t];if("code"===i.type){var r=i.content[1],o=i.content[3];if(r&&o&&"code-language"===r.type&&"code-block"===o.type&&"string"==typeof r.content){var l=r.content.replace(/\b#/g,"sharp").replace(/\b\+\+/g,"pp"),s="language-"+(l=(/[a-z][\w-]*/i.exec(l)||[""])[0].toLowerCase());o.alias?"string"==typeof o.alias?o.alias=[o.alias,s]:o.alias.push(s):o.alias=[s]}}else n(i.content)}}(n.tokens)})),n.hooks.add("wrap",(function(e){if("code-block"===e.type){for(var t="",a=0,i=e.classes.length;a<i;a++){var s=e.classes[a],d=/language-(.+)/.exec(s);if(d){t=d[1];break}}var p=n.languages[t];if(p)e.content=n.highlight(e.content.replace(r,"").replace(/&(\w{1,8}|#x?[\da-f]{1,8});/gi,(function(n,e){var t;return"#"===(e=e.toLowerCase())[0]?(t="x"===e[1]?parseInt(e.slice(2),16):Number(e.slice(1)),l(t)):o[e]||n})),p,t);else if(t&&"none"!==t&&n.plugins.autoloader){var u="md-"+(new Date).valueOf()+"-"+Math.floor(1e16*Math.random());e.attributes.id=u,n.plugins.autoloader.loadLanguages(t,(function(){var e=document.getElementById(u);e&&(e.innerHTML=n.highlight(e.textContent,n.languages[t],t))}))}}}));var r=RegExp(n.languages.markup.tag.pattern.source,"gi"),o={amp:"&",lt:"<",gt:">",quot:'"'},l=String.fromCodePoint||String.fromCharCode;n.languages.md=n.languages.markdown}(Prism);
!function(e){e.languages.typescript=e.languages.extend("javascript",{"class-name":{pattern:/(\b(?:class|extends|implements|instanceof|interface|new|type)\s+)(?!keyof\b)(?!\s)[_$a-zA-Z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*(?:\s*<(?:[^<>]|<(?:[^<>]|<[^<>]*>)*>)*>)?/,lookbehind:!0,greedy:!0,inside:null},builtin:/\b(?:Array|Function|Promise|any|boolean|console|never|number|string|symbol|unknown)\b/}),e.languages.typescript.keyword.push(/\b(?:abstract|declare|is|keyof|readonly|require)\b/,/\b(?:asserts|infer|interface|module|namespace|type)\b(?=\s*(?:[{_$a-zA-Z\xA0-\uFFFF]|$))/,/\btype\b(?=\s*(?:[\{*]|$))/),delete e.languages.typescript.parameter,delete e.languages.typescript["literal-property"];var s=e.languages.extend("typescript",{});delete s["class-name"],e.languages.typescript["class-name"].inside=s,e.languages.insertBefore("typescript","function",{decorator:{pattern:/@[$\w\xA0-\uFFFF]+/,inside:{at:{pattern:/^@/,alias:"operator"},function:/^[\s\S]+/}},"generic-function":{pattern:/#?(?!\s)[_$a-zA-Z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*\s*<(?:[^<>]|<(?:[^<>]|<[^<>]*>)*>)*>(?=\s*\()/,greedy:!0,inside:{function:/^#?(?!\s)[_$a-zA-Z\xA0-\uFFFF](?:(?!\s)[$\w\xA0-\uFFFF])*/,generic:{pattern:/<[\s\S]+/,alias:"class-name",inside:s}}}}),e.languages.ts=e.languages.typescript}(Prism);
</script>
  </body>
</html>"##;
        assert_eq!(html, expected_result);

        // cleanup
        remove_file(html_path).expect("Unable to delete HTML output in cleanup");
    }
}
