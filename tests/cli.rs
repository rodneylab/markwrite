use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;

#[test]
fn it_returns_error_when_input_file_does_not_exist() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("markwrite")?;

    cmd.arg("nonsense.md");
    cmd.assert().failure().stderr(predicate::str::contains(
        "Unable to open input (nonsense.md), check the path is correct.",
    ));

    Ok(())
}
