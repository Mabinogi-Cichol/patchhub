use serde_json::Value;
use steam_patch_installer_format::{audit, behavior, graph, lua, Result};

fn unhex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn diff(a: &Value, b: &Value, path: &str) -> Option<String> {
    if a == b {
        return None;
    }
    match (a, b) {
        (Value::Object(aa), Value::Object(bb)) => {
            if aa.keys().collect::<Vec<_>>() != bb.keys().collect::<Vec<_>>() {
                return Some(format!("{path}: keys {:?} != {:?}", aa.keys(), bb.keys()));
            }
            for (k, v) in aa {
                if let Some(d) = diff(v, &bb[k], &format!("{path}.{k}")) {
                    return Some(d);
                }
            }
        }
        (Value::Array(aa), Value::Array(bb)) => {
            if aa.len() != bb.len() {
                return Some(format!("{path}: len {} != {}", aa.len(), bb.len()));
            }
            for (i, (v, w)) in aa.iter().zip(bb).enumerate() {
                if let Some(d) = diff(v, w, &format!("{path}[{i}]")) {
                    return Some(d);
                }
            }
        }
        _ => return Some(format!("{path}: {a} != {b}")),
    }
    None
}
fn verify(fixture: &str, run: impl Fn(&Value) -> Result<Value>) {
    let golden: Value = serde_json::from_str(fixture).unwrap();
    for case in golden["cases"].as_array().unwrap() {
        let result = run(&case["input"]);
        if let Some(expected) = case.get("expected") {
            let actual = result.unwrap_or_else(|e| panic!("case {}: {e}", case["id"]));
            assert!(
                actual == *expected,
                "case {}: {}",
                case["id"],
                diff(&actual, expected, "$").unwrap()
            );
        } else {
            let error = result.expect_err(&format!("case {} must reject", case["id"]));
            assert_eq!(
                error.0,
                case["error"].as_str().unwrap(),
                "case {}",
                case["id"]
            );
        }
    }
}
#[test]
fn python_ast_golden() {
    verify(include_str!("fixtures/sf/lua-golden.json"), |i| {
        lua::parse(i["source"].as_str().unwrap())
    });
}
#[test]
fn python_behavior_golden() {
    verify(include_str!("fixtures/sf/behavior-golden.json"), |i| {
        behavior::lower(
            &i["graph"],
            i["files"].as_array().unwrap(),
            i["version"].as_str().unwrap(),
        )
    });
}
#[test]
fn python_graph_golden() {
    verify(include_str!("fixtures/sf/graph-golden.json"), |i| {
        graph::decode(
            &unhex(i["data_hex"].as_str().unwrap()),
            i["version"].as_str().unwrap(),
        )
    });
}
#[test]
fn python_inventory_golden() {
    verify(include_str!("fixtures/sf/inventory-golden.json"), |i| {
        graph::inventory(&unhex(i["data_hex"].as_str().unwrap()))
    });
}
#[test]
fn python_container_golden() {
    verify(include_str!("fixtures/sf/audit-golden.json"), |i| {
        audit::audit(&unhex(i["data_hex"].as_str().unwrap()), false)
    });
}
#[test]
fn python_full_behavior_report_golden() {
    let golden: Value =
        serde_json::from_str(include_str!("fixtures/sf/worker-golden.json")).unwrap();
    for case in golden["cases"].as_array().unwrap() {
        let report = audit::audit(&unhex(case["data_hex"].as_str().unwrap()), true).unwrap();
        assert!(
            report == case["report"],
            "case {}: {}",
            case["name"],
            diff(&report, &case["report"], "$").unwrap()
        );
    }
}
