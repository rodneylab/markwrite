mod html_process;
mod markdown;
mod url_utility;

use crate::html_process::process_html;
use anyhow::{Context, Result};
use log::info;
use markdown::{parse_markdown_to_html, Heading, TextStatistics};
use std::{
    fs::{read_to_string, File},
    io::Write,
    path::Path,
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

pub fn markdown_to_processed_html(markdown: &str, options: &ParseInputOptions) -> ParseResults {
    let markdown = strip_frontmatter(markdown);
    match parse_markdown_to_html(markdown) {
        Ok((html_value, headings, statistics_value)) => {
            let html = Some(process_html(
                &html_value,
                options.canonical_root_url.as_deref(),
                options.search_term.as_deref(),
            ));
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

pub fn update_html<P1: AsRef<Path>, P2: AsRef<Path>>(
    path: &P1,
    output_path: &P2,
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

    let ParseResults { html, .. } = markdown_to_processed_html(&markdown, &options);
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
        }
        None => eprintln!("[ ERROR ] Unable to parse markdownto HTML"),
    };
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::{strip_frontmatter, update_html};
    use std::{
        fs::{read_to_string, remove_file},
        path::Path,
    };

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
    fn update_html_writes_parsed_markdown_to_html_file() {
        // arrange
        let markdown_path = Path::new("./fixtures/file.md");
        let html_path = Path::new("./fixtures/file.html");

        // act
        update_html(&markdown_path, &html_path).expect("Error calling update_html");

        // assert
        let html = read_to_string(html_path).expect("Unable to read generated HTML");
        let expected_result = "<h1 id=\"test\">Test</h1>\n<p>This is a test.</p>\n";
        assert_eq!(html, expected_result);

        // cleanup
        remove_file(html_path).expect("Unable to delete HTML output in cleanup");
    }
}
