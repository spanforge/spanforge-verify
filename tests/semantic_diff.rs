use spanforge_verify::{
    privacy::Masker,
    semantic_diff::{Differences, Options},
};

#[test]
fn json_reports_escaped_pointers_missing_null_and_exact_large_numbers() {
    let masker = Masker::new([]).unwrap();
    let mut diff = Differences::new(&masker);
    let options = Options {
        json_stdout: true,
        ..Default::default()
    };
    diff.stream(
        "one",
        "stdout",
        br#"{"a/b":{"~k":9007199254740992},"null":null}"#,
        br#"{"a/b":{"~k":9007199254740993}}"#,
        &options,
    )
    .unwrap();
    assert_eq!(diff.items.len(), 2);
    assert_eq!(diff.items[0].path, "/a~1b/~0k");
    assert_ne!(diff.items[0].baseline, diff.items[0].candidate);
    assert_eq!(diff.items[1].kind, "removed");
    assert_eq!(diff.items[1].baseline.as_deref(), Some("null"));
    assert!(diff.items[1].candidate.is_none());
}

#[test]
fn crlf_is_explicit_and_binary_comparison_is_lossless() {
    let masker = Masker::new([]).unwrap();
    let mut diff = Differences::new(&masker);
    diff.stream("one", "stdout", b"a\r\n", b"a\n", &Options::default())
        .unwrap();
    assert_eq!(diff.items.len(), 1);
    let mut diff = Differences::new(&masker);
    diff.stream(
        "one",
        "stdout",
        b"a\r\n",
        b"a\n",
        &Options {
            crlf_to_lf: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(diff.items.is_empty());
    diff.stream("one", "stdout", &[0, 255], &[0, 254], &Options::default())
        .unwrap();
    assert_eq!(diff.items[0].path, "byte:1");
    assert!(diff.items[0].baseline.as_ref().unwrap().contains("binary"));
}

#[test]
fn caps_diffs_rejects_duplicate_json_and_masks_pointer_keys() {
    let masker = Masker::new(["secret/key".into()]).unwrap();
    let mut diff = Differences::new(&masker);
    let options = Options {
        json_stdout: true,
        ..Default::default()
    };
    diff.stream(
        "one",
        "stdout",
        br#"{"secret/key":1}"#,
        br#"{"secret/key":2}"#,
        &options,
    )
    .unwrap();
    assert_eq!(diff.items[0].path, "[REDACTED POINTER]");
    assert!(
        diff.stream("one", "stdout", br#"{"a":1,"a":2}"#, b"{}", &options)
            .is_err()
    );
    for i in 0..1005 {
        diff.add("one", "file", &i.to_string(), "changed", None, None);
    }
    assert_eq!(diff.items.len(), 1000);
    assert_eq!(diff.omitted, 6);
}

#[test]
fn text_excerpt_shows_deep_changes_and_never_splits_a_secret_before_masking() {
    let secret = "a-secret-spanning-the-diagnostic-boundary";
    let masker = Masker::new([secret.into()]).unwrap();
    let old = format!("{}old {secret}", "unchanged\n".repeat(1000));
    let new = format!("{}new {secret}", "unchanged\n".repeat(1000));
    let mut diff = Differences::new(&masker);
    diff.stream(
        "one",
        "stdout",
        old.as_bytes(),
        new.as_bytes(),
        &Options::default(),
    )
    .unwrap();
    let before = diff.items[0].baseline.as_ref().unwrap();
    let after = diff.items[0].candidate.as_ref().unwrap();
    assert!(before.contains("old [REDACTED]"));
    assert!(after.contains("new [REDACTED]"));
    assert!(!before.contains("a-secret"));
}

#[test]
fn a_breaking_change_after_diagnostic_cap_still_counts_as_blocking() {
    use spanforge_verify::compatibility::{Impact, Policy, Rule};
    let policy = Policy {
        schema_version: 1,
        name: "test-policy".into(),
        comparison: Options::default(),
        rules: vec![
            Rule {
                id: "important".into(),
                case: None,
                field: "stdout".into(),
                path: Some("/1000".into()),
                impact: Impact::Breaking,
                reason: "Must remain stable".into(),
            },
            Rule {
                id: "allowed".into(),
                case: None,
                field: "stdout".into(),
                path: None,
                impact: Impact::Allowed,
                reason: "May change".into(),
            },
        ],
    };
    policy.validate().unwrap();
    let masker = Masker::new([]).unwrap();
    let mut differences = Differences::with_policy(&masker, Some(&policy));
    for i in 0..1001 {
        differences.add("one", "stdout", &format!("/{i}"), "changed", None, None);
    }
    assert_eq!(differences.items.len(), 1000);
    assert_eq!(differences.omitted, 1);
    assert_eq!(differences.allowed, 1000);
    assert_eq!(differences.blocking, 1);
    assert_eq!(differences.breaking, 1);
}

#[test]
fn masks_original_secret_before_crlf_normalization_in_diagnostics() {
    let masker = Masker::new(["secret\r\nvalue".into()]).unwrap();
    let mut differences = Differences::new(&masker);
    let options = Options {
        crlf_to_lf: true,
        ..Default::default()
    };
    differences
        .stream(
            "one",
            "stdout",
            b"old secret\r\nvalue",
            b"new secret\r\nvalue",
            &options,
        )
        .unwrap();
    for summary in [
        &differences.items[0].baseline,
        &differences.items[0].candidate,
    ]
    .into_iter()
    .flatten()
    {
        assert!(summary.contains("[REDACTED]"));
        assert!(!summary.contains("secret\nvalue"));
    }
}
