use serde_json::Value;
use std::{env, fmt::Write, fs, path::PathBuf};

fn read_input(path: &str) -> String {
    println!("cargo:rerun-if-changed={path}");
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn main() {
    let mut config: Value =
        serde_json::from_str(&read_input("resources/sources.json")).expect("parse bundled sources");
    let scripts = config
        .as_object_mut()
        .expect("source config object")
        .remove("bundled_scripts")
        .expect("bundled script declarations");
    let scripts = scripts.as_object().expect("bundled_scripts object");
    let sources = config["sources"].as_array().expect("source rules");
    for source in sources {
        let adapter = source["adapter"].as_str().expect("bundled source adapter");
        assert!(scripts.contains_key(adapter), "no script for {adapter}");
    }

    let mut generated =
        String::from("pub fn code(adapter: &str) -> Option<&'static str> {\n    match adapter {\n");
    for (adapter, file) in scripts {
        let file = file.as_str().expect("Lua filename");
        let code = read_input(&format!("resources/source-scripts/{file}"));
        writeln!(generated, "        {adapter:?} => Some({code:?}),").unwrap();
    }
    generated.push_str("        _ => None,\n    }\n}\n");
    let discovery_adapters: Vec<_> = sources
        .iter()
        .filter(|source| source["discovery_only"] == true)
        .map(|source| source["adapter"].as_str().expect("bundled source adapter"))
        .collect();
    writeln!(
        generated,
        "pub const DISCOVERY_ADAPTERS: &[&str] = &{discovery_adapters:?};"
    )
    .unwrap();
    writeln!(
        generated,
        "pub const DEFAULT_RULES_JSON: &str = {:?};",
        config.to_string()
    )
    .unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    fs::write(output.join("bundled_sources.rs"), generated).expect("write bundled sources");
}
