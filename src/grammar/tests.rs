use crate::grammar::GrammarCheckResult;

#[test]
fn test_context() {
    //arrange

    let grammar_check_result = GrammarCheckResult {
        context_length: 4,
        context_offset: 16,
        message: "Possible spelling mistake found.".into(),
        sentence: "The quick brown foox jumps over the lazy dog".into(),
        short_message: "Spelling mistake".into(),
        text: "The quick brown foox jumps over the lazy dog".into(),
        replacements: vec![
            "food".into(),
            "foot".into(),
            "fool".into(),
            "fox".into(),
            "foo".into(),
        ],
    };

    // act
    let result = grammar_check_result.context();

    // assert
    let expected = "The quick brown \u{1b}[94mfoox\u{1b}[39m jumps over the lazy dog";
    assert_eq!(result, expected);
}
