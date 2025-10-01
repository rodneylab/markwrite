#[cfg(test)]
mod tests;

use log::trace;
use owo_colors::{
    colors::{BrightBlue, BrightCyan, White},
    OwoColorize,
};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt::Write};

#[derive(Debug)]
pub struct CheckResult {
    context_length: u32,
    context_offset: u32,
    message: String,
    sentence: String,
    short_message: String,
    text: String,
    replacements: Vec<String>,
}

impl CheckResult {
    pub fn context(&self) -> String {
        let CheckResult {
            context_length,
            context_offset,
            ..
        } = &self;

        let highlight_start: usize = (*context_offset)
            .try_into()
            .expect("Error forming highlight string: unable to convert integer type");
        let highlight_end: usize = highlight_start
            + <u32 as TryInto<usize>>::try_into(*context_length)
                .expect("Error forming highlight string: unable to convert integer type");
        format!(
            "{}{}{}",
            &self.text[..highlight_start],
            &self.text[highlight_start..highlight_end]
                .to_string()
                .fg::<BrightBlue>(),
            &self.text[highlight_end..],
        )
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn replacements_string(&self) -> Option<String> {
        if self.replacements.is_empty() {
            None
        } else {
            Some(
                self.replacements
                    .iter()
                    .fold(String::new(), |mut output, val| {
                        let _ = writeln!(
                            output,
                            "        {} {}",
                            "-".to_string().fg::<White>(),
                            val.to_string().fg::<BrightCyan>(),
                        );
                        output
                    }),
            )
        }
    }

    pub fn sentence(&self) -> &str {
        &self.sentence
    }

    pub fn short_message(&self) -> &str {
        &self.short_message
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponseDetectedLanguage {
    #[allow(dead_code)]
    name: String,

    #[allow(dead_code)]
    code: String,

    #[allow(dead_code)]
    confidence: f64,

    #[allow(dead_code)]
    source: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponseLanguage {
    #[allow(dead_code)]
    name: String,

    #[allow(dead_code)]
    code: String,

    #[allow(dead_code)]
    detected_language: LanguageToolsCheckResponseDetectedLanguage,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponseMatchContext {
    text: String,
    offset: u32,
    length: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponseMatchReplacement {
    value: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponseMatchType {
    type_name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponseMatchRuleCategory {
    id: String,
    name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponseMatchRule {
    id: String,
    description: String,
    issue_type: String,
    category: LanguageToolsCheckResponseMatchRuleCategory,
    is_premium: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponseMatch {
    message: String,
    short_message: String,
    replacements: Vec<LanguageToolsCheckResponseMatchReplacement>,
    offset: u32,
    length: u32,
    context: LanguageToolsCheckResponseMatchContext,
    sentence: String,

    #[serde(rename(deserialize = "type", serialize = "type"))]
    match_type: LanguageToolsCheckResponseMatchType,
    rule: LanguageToolsCheckResponseMatchRule,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponseSoftware {
    #[allow(dead_code)]
    name: String,

    #[allow(dead_code)]
    version: String,

    #[allow(dead_code)]
    build_date: String,

    #[allow(dead_code)]
    api_version: u32,

    #[allow(dead_code)]
    premium: bool,

    #[allow(dead_code)]
    premium_hint: String,

    #[allow(dead_code)]
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponseWarnings {
    #[allow(dead_code)]
    incomplete_results: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanguageToolsCheckResponse {
    #[allow(dead_code)]
    software: LanguageToolsCheckResponseSoftware,

    #[allow(dead_code)]
    warnings: LanguageToolsCheckResponseWarnings,

    #[allow(dead_code)]
    language: LanguageToolsCheckResponseLanguage,
    matches: Vec<LanguageToolsCheckResponseMatch>,
    sentence_ranges: Vec<Vec<u32>>,
}

pub struct Checker<'a> {
    url: &'a str,
}

impl Checker<'_> {
    pub fn new(url: Option<&str>) -> Checker<'_> {
        let actual_url: &str = match url {
            Some(value) => value,
            None => "https://api.languagetoolplus.com/v2/check",
        };
        Checker { url: actual_url }
    }

    fn process_language_tools_results(
        response: &LanguageToolsCheckResponse,
        results: &mut Vec<CheckResult>,
    ) {
        let LanguageToolsCheckResponse {
            matches,
            sentence_ranges,
            ..
        } = response;

        for results_match in matches {
            let LanguageToolsCheckResponseMatch {
                context,
                message,
                replacements,
                sentence,
                short_message,
                ..
            } = &results_match;
            let LanguageToolsCheckResponseMatchContext {
                length,
                offset,
                text,
            } = context;
            let replacements = if replacements.len() < 5 {
                replacements
            } else {
                &replacements[0..5]
            };
            let replacements_vec = replacements
                .iter()
                .map(|val| {
                    let LanguageToolsCheckResponseMatchReplacement { value } = val;
                    &value[..]
                })
                .collect::<Vec<&str>>();
            trace!(
                "Match: {}",
                &serde_json::to_string_pretty(&results_match).unwrap()
            );
            results.push(CheckResult {
                context_length: *length,
                context_offset: *offset,
                message: message.to_string(),
                short_message: short_message.to_string(),
                text: text.to_string(),
                replacements: replacements_vec
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect(),
                sentence: sentence.to_string(),
            });
        }
        trace!(
            "Sentence ranges: {}",
            &serde_json::to_string_pretty(&sentence_ranges).unwrap()
        );
    }

    pub async fn check_chunk(
        &self,
        text: &str,
    ) -> Result<Vec<CheckResult>, Box<dyn std::error::Error>> {
        let mut results = Vec::new();
        let client = reqwest::Client::new();
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_str("application/json").expect("Expected valid accept header value"),
        );
        let mut body_data_map = HashMap::new();
        body_data_map.insert("text", text);
        body_data_map.insert("language", "en-GB");
        body_data_map.insert("level", "picky");

        let languagetool_response_data = match client
            .post(self.url)
            .headers(headers)
            .form(&body_data_map)
            .send()
            .await
        {
            Ok(response_value) => match response_value.json::<LanguageToolsCheckResponse>().await {
                Ok(json_value) => json_value,
                Err(error) => {
                    if !error.is_request() && error.is_body() {
                        eprintln!(
                        "[ ERROR ] error receiving response from remote grammar server response: {error:?}.",
                    );
                        return Err(error.into());
                    }
                    eprintln!("[ ERROR ] error parsing remote grammar server response: {error:?}.",);
                    return Err(error.into());
                }
            },
            Err(error) => {
                eprintln!("[ ERROR ] no response from remote grammar check server: {error:?}.",);
                return Err(error.into());
            }
        };
        Self::process_language_tools_results(&languagetool_response_data, &mut results);
        Ok(results)
    }
}
