use spanforge_verify::model::{RunResult, Status};
#[test]
fn wire_contracts_round_trip_with_required_nulls() {
    for name in [
        "pass",
        "target-fail",
        "cancellation",
        "configuration-error",
        "spawn-failure",
        "mixed-abort",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/reports/{name}.json"));
        let bytes = std::fs::read(path).unwrap();
        let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let result: RunResult = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(result.schema_version, 1);
        assert_eq!(result.status.exit_code(), result.exit_code);
        assert_eq!(serde_json::to_value(&result).unwrap(), original, "{name}");
    }
}
#[test]
fn aggregate_precedence_retains_infrastructure_abort() {
    assert_eq!(
        Status::aggregate([Status::Fail, Status::Inconclusive, Status::InfraError]),
        Status::InfraError
    );
    assert_eq!(
        Status::aggregate([Status::Pass, Status::Fail]),
        Status::Fail
    );
    assert_eq!(
        Status::aggregate([Status::Fail, Status::Inconclusive]),
        Status::Inconclusive
    );
    assert_eq!(Status::aggregate([]), Status::ConfigError);
}

#[test]
fn junit_fixtures_have_matching_counts_and_status_mapping() {
    use quick_xml::{Reader, events::Event};
    for name in [
        "pass",
        "target-fail",
        "cancellation",
        "configuration-error",
        "spawn-failure",
        "mixed-abort",
    ] {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reports");
        let json: RunResult =
            serde_json::from_slice(&std::fs::read(root.join(format!("{name}.json"))).unwrap())
                .unwrap();
        let xml = std::fs::read_to_string(root.join(format!("{name}.xml"))).unwrap();
        let mut reader = Reader::from_str(&xml);
        let mut declared = [0usize; 4];
        let mut actual = [0usize; 4];
        loop {
            match reader.read_event().unwrap() {
                Event::Start(node) | Event::Empty(node) => match node.name().as_ref() {
                    "testsuite" => {
                        for attr in node.attributes() {
                            let attr = attr.unwrap();
                            let index = match attr.key.as_ref() {
                                "tests" => Some(0),
                                "failures" => Some(1),
                                "errors" => Some(2),
                                "skipped" => Some(3),
                                _ => None,
                            };
                            if let Some(index) = index {
                                declared[index] = attr.value.parse().unwrap();
                            }
                        }
                    }
                    "testcase" => actual[0] += 1,
                    "failure" => actual[1] += 1,
                    "error" => actual[2] += 1,
                    "skipped" => actual[3] += 1,
                    _ => (),
                },
                Event::Eof => break,
                _ => (),
            }
        }
        assert_eq!(actual, declared, "{name}");
        assert_eq!(actual[0], json.cases.len() + json.errors.len());
        assert_eq!(
            actual[1],
            json.cases
                .iter()
                .filter(|c| c.status == Status::Fail)
                .count()
        );
        assert_eq!(
            actual[2],
            json.cases
                .iter()
                .filter(|c| c.status == Status::InfraError)
                .count()
                + json.errors.len()
        );
        assert_eq!(
            actual[3],
            json.cases
                .iter()
                .filter(|c| c.status == Status::Inconclusive)
                .count()
        );
    }
}
