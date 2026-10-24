use patchhub_installer_format::{audit, graph};
use serde_json::Value;

fn fixture() -> (Vec<u8>, String) {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/sf/graph-golden.json")).unwrap();
    let case = fixtures["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c.get("expected").is_some())
        .unwrap();
    let bytes = case["input"]["data_hex"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect();
    (bytes, case["input"]["version"].as_str().unwrap().to_owned())
}

#[test]
fn file_tail_is_data_and_does_not_shift_the_next_object() {
    let (input, version) = fixture();
    let (rows, _) = audit::file_table(&input, &version).unwrap();
    let end = (rows[0]["serialized_record"]["offset"].as_u64().unwrap()
        + rows[0]["serialized_record"]["size"].as_u64().unwrap()) as usize;
    let expected = graph::decode(&input, &version).unwrap();
    for flag in [0, 1, 17, 255] {
        let mut changed = input.clone();
        changed[end] = flag;
        let parsed = graph::decode(&changed, &version).unwrap();
        assert_eq!(parsed["coverage"], expected["coverage"]);
        assert_eq!(parsed["objects"], expected["objects"]);
        assert_eq!(parsed["events"], expected["events"]);
        assert_eq!(audit::file_table(&changed, &version).unwrap().0, rows);
    }
}

#[test]
fn ordinary_project_options_are_not_compared_to_a_default_byte_pattern() {
    let (input, version) = fixture();
    let pattern: Vec<u8> = [1u32, 0, 0, 30, 30, 1]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let offset = input
        .windows(pattern.len())
        .position(|w| w == pattern)
        .unwrap();
    let mut changed = input.clone();
    changed[offset..offset + pattern.len()].copy_from_slice(
        &[9u32, 7, 3, 45, 60, 255]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let parsed = graph::decode(&changed, &version).unwrap();
    assert_eq!(parsed["coverage"]["bytes"], input.len());
    assert_eq!(parsed["coverage"]["complete"], true);
    assert_eq!(
        audit::file_table(&changed, &version).unwrap().0,
        audit::file_table(&input, &version).unwrap().0
    );
}
