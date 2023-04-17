use log::trace;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::Write};

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

pub struct GrammarChecker<'a> {
    url: &'a str,
}

impl<'a> GrammarChecker<'a> {
    pub fn new(url: Option<&str>) -> GrammarChecker {
        let actual_url: &str = match url {
            Some(value) => value,
            None => "https://api.languagetoolplus.com/v2/check",
        };
        GrammarChecker { url: actual_url }
    }

    pub async fn check_chunk(&self, text: &str, stdout_handle: &mut impl Write) {
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
        writeln!(
            stdout_handle,
            "Checking chunk spelling, punctuation and grammar..."
        )
        .expect("Expected to be able to write to stdout");
        stdout_handle
            .flush()
            .expect("Expected to be able to flush stdout");
        match client
            .post(self.url)
            .headers(headers)
            .form(&body_data_map)
            .send()
            .await
        {
            Ok(response_value) => match response_value.json::<LanguageToolsCheckResponse>().await {
                Ok(json_value) => {
                    let LanguageToolsCheckResponse {
                        matches,
                        sentence_ranges,
                        ..
                    } = json_value;

                    for results_match in matches {
                        let LanguageToolsCheckResponseMatch {
                            context,
                            rule,
                            match_type,
                            replacements,
                            ..
                        } = &results_match;
                        let LanguageToolsCheckResponseMatchContext { text, .. } = context;
                        let LanguageToolsCheckResponseMatchRule { description, .. } = rule;
                        writeln!(stdout_handle, "/nText: {text}")
                            .expect("Expected to be able to write to stdout");
                        writeln!(stdout_handle, "Rule: {description}")
                            .expect("Expected to be able to write to stdout");

                        let LanguageToolsCheckResponseMatchType { type_name, .. } = match_type;
                        if type_name == "UnknownWord" {
                            let replacements = if replacements.len() < 5 {
                                replacements
                            } else {
                                &replacements[0..5]
                            };
                            let replacements_string = replacements
                                .iter()
                                .map(|val| {
                                    let LanguageToolsCheckResponseMatchReplacement { value } = val;
                                    &value[..]
                                })
                                .collect::<Vec<&str>>()
                                .join(", ");
                            writeln!(stdout_handle, "Replacements: {replacements_string}.",)
                                .expect("Expected to be able to write to stdout");
                        };
                        trace!(
                            "Match: {}",
                            &serde_json::to_string_pretty(&results_match).unwrap()
                        );
                    }
                    trace!(
                        "Sentence ranges: {}",
                        &serde_json::to_string_pretty(&sentence_ranges).unwrap()
                    );
                }
                Err(e) => eprintln!(
                    "[ ERROR ] error parsing remote grammar server response: {:?}.",
                    e
                ),
            },
            Err(e) => eprintln!(
                "[ ERROR ] no response from remote grammar check server: {:?}.",
                e
            ),
        };
    }
}
