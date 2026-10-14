use serde_json::{json, Value};
use steam_patch_installer_format::{behavior, lua};

fn checked_file(version: &str) -> Value {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut timestamp = vec![10, 0, 0, 128];
    timestamp.extend([0; 8]);
    timestamp.extend([10, 0, 0, 128]);
    let mut mask = vec![0, 128];
    mask.extend(vec![255; if version == "9.5" { 30 } else { 36 }]);
    let mut flag = vec![2];
    flag.extend([0; 10]);
    let fields = vec![
        vec![1, 0],
        vec![10, 0, 0, 128],
        timestamp,
        vec![0; 25],
        flag,
        vec![0],
        vec![0; 8],
        vec![0; if version == "9.7" { 4 } else { 3 }],
        vec![0, 0, 0, 0, 232, 3, 0, 0, 0, 0],
        mask,
        vec![1, 0],
    ];
    json!({"destination":"payload.bin","runtime_folder":"Archive","condition":"","install_type":"All","extra":"None","packages":[],
        "shortcuts":["","","","",""],"icon":"","font":"","attributes":32,"use_original_attributes":1,"forced_attributes":0,
        "unpacked_size":42,"unreviewed_fields":fields.into_iter().map(|f|json!({"base64":STANDARD.encode(f)})).collect::<Vec<_>>()})
}
fn graph(source: &str) -> Value {
    json!({"coverage":{"complete":true},"includes":[],"terms":[],"events":[{"context":"CSUFScreenInstallFolder:test","name":"On Next","script":source}]})
}
fn lower(source: &str) -> steam_patch_installer_format::Result<Value> {
    behavior::lower(&graph(source), &[checked_file("9.5")], "9.5")
}

#[test]
fn literals_comments_and_operand_results() {
    assert_eq!(
        lua::parse("local x=0x10; local s='a\\n\\065'; --[=[ignored]=]\nScreen.Next()").unwrap()[0],
        json!(["assign", [["name", "x"]], [["literal", 16]], true])
    );
    let result=lower("local value=true and 'original'; local fallback=false or 'fallback'; if value=='original' and fallback=='fallback' then Screen.Next() end").unwrap();
    assert_eq!(result["condition"], true);
    assert_eq!(
        lower("-- safe\nFile.Run('bad')").unwrap_err().0,
        "exe_lua_call_unsupported:File.Run"
    );
    assert_eq!(
        lower("if false then File.Run('bad') else Screen.Next() end").unwrap()["condition"],
        true
    );
}
#[test]
fn semantic_conditions_and_locator_classification() {
    let result=lower(r#"if File.DoesExist("%AppFolder%\\engine.exe") and File.GetCRC("%AppFolder%\\identity.dat")==123 then Screen.Next() end"#).unwrap();
    assert_eq!(
        result["condition"],
        json!([
            "symbol",
            "and",
            ["symbol", "exists", "engine.exe"],
            ["symbol", "==", ["symbol", "crc32", "identity.dat"], 123]
        ])
    );
    let mut locator = graph(
        r#"local path=Registry.GetValue(1,"key","value"); if File.DoesExist(path.."\\engine.exe") then SessionVar.Set("%AppFolder%",path); Screen.Next() end"#,
    );
    assert_eq!(
        behavior::lower(&locator, &[checked_file("9.5")], "9.5")
            .unwrap_err()
            .0,
        "exe_registry_read_unsupported"
    );
    locator["events"][0]["context"] = json!("CSUFScreenStaticText:welcome");
    assert_eq!(
        behavior::lower(&locator, &[checked_file("9.5")], "9.5").unwrap()["condition"],
        json!(["symbol", "exists", "engine.exe"])
    );
}
#[test]
fn includes_function_results_and_global_writes() {
    let mut value = graph("if probe('%AppFolder%') then Screen.Next() end");
    value["includes"] = json!([{"script":r#"function probe(root) return File.DoesExist(root.."\\engine.exe") end"#}]);
    assert_eq!(
        behavior::lower(&value, &[checked_file("9.5")], "9.5").unwrap()["condition"],
        json!(["symbol", "exists", "engine.exe"])
    );
    value["includes"] = json!([{"script":"function probe(root) if File.DoesExist('%AppFolder%\\\\engine.exe') then return true else return false end end"}]);
    assert_eq!(
        behavior::lower(&value, &[checked_file("9.5")], "9.5")
            .unwrap_err()
            .0,
        "exe_lua_function_result_unsupported"
    );
    value["includes"] = json!([{"script":"function probe(root) if File.DoesExist('%AppFolder%\\\\engine.exe') then new_global=1 else new_global=2 end end"}]);
    assert_eq!(
        behavior::lower(&value, &[checked_file("9.5")], "9.5")
            .unwrap_err()
            .0,
        "exe_lua_global_mutation_unsupported"
    );
}
#[test]
fn runtime_guards_progress_and_unknown_effects_fail_closed() {
    let mut value = graph("Screen.Next()");
    value["events"]
        .as_array_mut()
        .unwrap()
        .push(json!({"context":"project","name":"On Startup","script":"_NeedsReboot=true"}));
    assert_eq!(
        behavior::lower(&value, &[checked_file("9.5")], "9.5")
            .unwrap_err()
            .0,
        "exe_lua_runtime_mutation_unsupported"
    );
    value["events"][1] = json!({"context":"CSUFScreenProgressBars:progress","name":"On Progress","script":"if e_Stage==3 then File.Run('bad') end"});
    assert_eq!(
        behavior::lower(&value, &[checked_file("9.5")], "9.5")
            .unwrap_err()
            .0,
        "exe_lua_call_unsupported:File.Run"
    );
    for source in [
        "File.Delete('bad')",
        "local f=File.Run; f('bad')",
        "while true do Screen.Next() end",
        "for i=1,2 do Screen.Next() end",
        "_G['File'].Run('bad')",
    ] {
        assert!(lower(source).is_err(), "{source}");
    }
}
#[test]
fn complete_structure_is_required_but_configuration_values_are_not_whitelisted() {
    let mut value = graph("Screen.Next()");
    value["coverage"]["complete"] = json!(false);
    assert_eq!(
        behavior::lower(&value, &[checked_file("9.5")], "9.5")
            .unwrap_err()
            .0,
        "exe_serialized_object_graph_incomplete"
    );
    for version in ["9.5", "9.6", "9.7"] {
        let file = checked_file(version);
        assert!(behavior::file_policy(std::slice::from_ref(&file), version).is_ok());
        for i in 0..11 {
            let mut changed = file.clone();
            changed["unreviewed_fields"][i]["base64"] = json!("AQ==");
            assert!(
                behavior::file_policy(&[changed], version).is_ok(),
                "field {i} {version}"
            );
        }
        for destination in [
            "SAVE/file.dat",
            "PROFILE.SAV",
            "savedata/game.dat",
            "uſerdata/config.bin",
        ] {
            let mut changed = file.clone();
            changed["destination"] = json!(destination);
            assert!(
                behavior::file_policy(&[changed], version).is_ok(),
                "{destination}"
            );
        }
    }
}
#[test]
fn syntax_and_call_bounds_are_enforced() {
    for source in ["if true then", "'unterminated", "--[[ unfinished"] {
        assert!(lua::parse(source).is_err());
    }
    assert_eq!(
        lua::parse(&("(".repeat(100) + "true" + &")".repeat(100)))
            .unwrap_err()
            .0,
        "exe_lua_depth_limit"
    );
    assert_eq!(
        lua::parse(&" ".repeat(1024 * 1024 + 1)).unwrap_err().0,
        "exe_lua_size_limit"
    );
    let mut value = graph("recursive()");
    value["includes"] = json!([{"script":"function recursive() return recursive() end"}]);
    assert_eq!(
        behavior::lower(&value, &[checked_file("9.5")], "9.5")
            .unwrap_err()
            .0,
        "exe_lua_call_depth_limit"
    );
    let source = (0..11)
        .map(|_| {
            "if File.DoesExist('%AppFolder%\\\\engine.exe') then local a=1 else local a=2 end;"
        })
        .collect::<String>()
        + "Screen.Next()";
    assert_eq!(lower(&source).unwrap_err().0, "exe_lua_branch_limit");
}
#[test]
fn iterative_ast_and_symbolic_expansion_cannot_escape_bounds() {
    for source in [
        format!("local x={}", "a.".repeat(110) + "a"),
        format!("local x={}", "1+".repeat(110) + "1"),
    ] {
        assert_eq!(lua::parse(&source).unwrap_err().0, "exe_lua_depth_limit");
    }
    let too_many_tokens = "local x=1;".repeat(20_001);
    assert_eq!(
        lua::parse(&too_many_tokens).unwrap_err().0,
        "exe_lua_token_limit"
    );
    let mut source="local a=File.DoesExist('%AppFolder%\\\\a.exe'); local b=File.DoesExist('%AppFolder%\\\\b.exe'); local x=a;".to_string();
    source.push_str(&"x=(a and x) or (b and x);".repeat(20));
    source.push_str("if x then Screen.Next() end");
    assert_eq!(lower(&source).unwrap_err().0, "exe_condition_ir_limit");
}
#[test]
fn integer_comparisons_preserve_precision_above_f64_range() {
    assert_eq!(
        lower("if 9007199254740993 > 9007199254740992 then Screen.Next() end").unwrap()
            ["condition"],
        true
    );
    assert_eq!(
        lower("if 9007199254740993 > 9007199254740992.0 then Screen.Next() end").unwrap()
            ["condition"],
        true
    );
    assert_eq!(
        lower("if -9007199254740993 < -9007199254740992.0 then Screen.Next() end").unwrap()
            ["condition"],
        true
    );
    assert_eq!(
        lower("if -1.5 < -1 and -2 < -1.5 then Screen.Next() end").unwrap()["condition"],
        true
    );
}
#[test]
fn precision_guards_cannot_hide_reachable_effects() {
    assert_eq!(
        lower(
            "if 9007199254740992 >= 9007199254740993 then Screen.Next() else File.Run('bad') end"
        )
        .unwrap_err()
        .0,
        "exe_lua_call_unsupported:File.Run"
    );
    let mut value = graph("guard=9007199254740992.0; Screen.Next()");
    value["includes"] = json!([{"script":"guard=9007199254740993"}]);
    assert_eq!(
        behavior::lower(&value, &[checked_file("9.5")], "9.5")
            .unwrap_err()
            .0,
        "exe_lua_runtime_mutation_unsupported"
    );
}
#[test]
fn global_scope_copy_behavior_matches_reference() {
    for global in [
        "guard=1; if true then guard=2 end",
        "guard=1; do guard=2 end",
        "guard=1; if true then guard=2 end; guard=3",
    ] {
        let mut value = graph("if guard==1 then Screen.Next() else File.Run('bad') end");
        value["includes"] = json!([{"script":global}]);
        assert_eq!(
            behavior::lower(&value, &[checked_file("9.5")], "9.5").unwrap()["condition"],
            true,
            "{global}"
        );
    }
}
#[test]
fn all_reviewed_literal_string_patterns_match_reference() {
    for (pattern, found) in [
        (r#"^\\[^/%*%?"<>|:\][^/%*%?"<>|:]+$"#, false),
        (r#"^%a:\[^/%*%?"<>|:]*$"#, true),
        (r#"\\"#, false),
        (r#"[/%*%?"<>|]"#, false),
        (r#"[/%*%?"<>|:]"#, false),
    ] {
        let source=format!("if (string.find(strInstallFolderPath,[=[{pattern}]=]) ~= nil) == {found} then Screen.Next() else File.Run('bad') end");
        assert_eq!(lower(&source).unwrap()["condition"], true, "{pattern}");
    }
}
#[test]
fn immutable_aliases_share_storage_and_ui_updates_are_bounded() {
    let source = format!(
        "local text=[=[{}]=];{}Screen.Next()",
        "a".repeat(512 * 1024),
        (0..10_000)
            .map(|i| format!("local alias{i}=text;"))
            .collect::<String>()
    );
    assert_eq!(lower(&source).unwrap()["condition"], true);
    let mut source = format!(
        "local props={{{}}};",
        (0..2048).map(|i| format!("v{i}={i},")).collect::<String>()
    );
    source.push_str(
        &(0..4000)
            .map(|i| format!("local alias{i}=props;"))
            .collect::<String>(),
    );
    source.push_str("if File.DoesExist('%AppFolder%\\\\engine.exe') then alias0.Text='yes' else alias1.Text='no' end;Screen.Next()");
    assert!(lower(&source).is_ok());
    assert_eq!(
        lower("local t={};t.Text=t;Screen.Next()").unwrap_err().0,
        "exe_lua_assignment_unsupported"
    );
    let source = format!(
        "local text=[=[{}]=];{}Screen.Next()",
        "a".repeat(512 * 1024),
        (0..160)
            .map(|i| format!("local alias{i}=text..'a';"))
            .collect::<String>()
    );
    assert_eq!(lower(&source).unwrap_err().0, "exe_lua_allocation_limit");
}
#[test]
fn table_dags_are_bounded_before_structural_comparisons() {
    let source = "local a={}; local b={};".to_string()
        + &"a={Text=a,Checked=a};b={Text=b,Checked=b};".repeat(20)
        + "if a ~= b then Screen.Next() else File.Run('bad') end";
    assert_eq!(lower(&source).unwrap_err().0, "exe_lua_table_limit");
}
#[test]
fn global_immutable_aliases_do_not_copy_environment_per_statement() {
    let source = format!(
        "local text=[=[{}]=];{}",
        "a".repeat(512 * 1024),
        (0..10_000)
            .map(|i| format!("local alias{i}=text;"))
            .collect::<String>()
    );
    let mut value = graph("Screen.Next()");
    value["includes"] = json!([{"script":source}]);
    assert_eq!(
        behavior::lower(&value, &[checked_file("9.5")], "9.5").unwrap()["condition"],
        true
    );
}

#[test]
fn ordinary_configuration_and_nested_conditions_are_compatible() {
    let mut file = checked_file("9.5");
    file["runtime_folder"] = json!(r"Archive\www\images");
    file["attributes"] = json!(7);
    file["use_original_attributes"] = json!(0);
    file["forced_attributes"] = json!(128);
    file["shortcuts"] = json!(["Desktop", "Game", "", "", ""]);
    file["icon"] = json!("custom.ico");
    file["font"] = json!("publisher.ttf");
    file["install_type"] = json!("Custom build");
    file["extra"] = json!("Publisher config");
    file["destination"] = json!("savedata/template.dat");
    let g = graph(r#"if File.DoesExist("%AppFolder%\\sub\\engine.exe") then Screen.Next() end"#);
    let result = behavior::lower(&g, &[file], "9.5").unwrap();
    assert_eq!(result["copy_files"], json!(["savedata/template.dat"]));
    assert_eq!(
        result["condition"],
        json!(["symbol", "exists", "sub/engine.exe"])
    );
}

#[test]
fn optional_files_and_package_install_state_are_real_selection_inputs() {
    let mut first = checked_file("9.5");
    first["destination"] = json!("optional.bin");
    first["condition"] = json!(r#"File.DoesExist("%AppFolder%\\dlc\\identity.dat")"#);
    let mut second = checked_file("9.5");
    second["destination"] = json!("component.bin");
    second["packages"] = json!(["off", "on"]);
    let mut g = graph("Screen.Next()");
    g["objects"] = json!([
        {"class":"CSUFPackage","name":"off","install":false,"enabled":true},
        {"class":"CSUFPackage","name":"on","install":true,"enabled":false}
    ]);
    let result = behavior::lower(&g, &[first.clone(), second.clone()], "9.5").unwrap();
    assert_eq!(result["condition"], true);
    assert_eq!(
        result["file_conditions"],
        json!([
            {"path":"optional.bin","condition":["symbol","exists","dlc/identity.dat"]}
        ])
    );
    second["packages"] = json!(["off"]);
    first["condition"] = json!("false");
    let result = behavior::lower(&g, &[first, second], "9.5").unwrap();
    assert_eq!(
        result["file_conditions"],
        json!([
            {"path":"optional.bin","condition":false},
            {"path":"component.bin","condition":false}
        ])
    );
    g["events"][0]["script"] =
        json!("SetupData.SetPackageProperties('off', {Install=true}); Screen.Next()");
    let mut file = checked_file("9.5");
    file["packages"] = json!(["off"]);
    assert_eq!(
        behavior::lower(&g, &[file], "9.5").unwrap()["file_conditions"],
        json!([])
    );
}

#[test]
fn package_properties_table_and_install_lifecycle_are_respected() {
    let mut g=graph("local p=SetupData.GetPackageProperties('component'); p.Install=true; SetupData.SetPackageProperties('component',p); Screen.Next()");
    g["objects"] = json!([{"class":"CSUFPackage","name":"component","install":false}]);
    let mut file = checked_file("9.5");
    file["packages"] = json!(["component"]);
    assert_eq!(
        behavior::lower(&g, &[file.clone()], "9.5").unwrap()["file_conditions"],
        json!([])
    );
    for (context, event) in [
        ("project", "On Post Install"),
        ("project", "On Shutdown"),
        ("uninstall", "On Preload"),
    ] {
        let mut changed = g.clone();
        changed["events"]
            .as_array_mut()
            .unwrap()
            .push(json!({"context":context,"name":event,
            "script":"SetupData.SetPackageProperties('component',{Install=false})"}));
        assert_eq!(
            behavior::lower(&changed, &[file.clone()], "9.5").unwrap()["file_conditions"],
            json!([]),
            "{context} {event}"
        );
    }
}

#[test]
fn project_startup_precedes_screens_and_help_does_not_choose_components() {
    let mut g = graph("SetupData.SetPackageProperties('component',{Install=true}); Screen.Next()");
    g["events"][0]["context"] = json!("CSUFScreenInstallFolder:first");
    g["objects"] = json!([{"class":"CSUFPackage","name":"component","install":true}]);
    g["events"].as_array_mut().unwrap().extend([
        json!({"context":"CSUFScreenInstallFolder:later","name":"On Help","script":"SetupData.SetPackageProperties('component',{Install=false})"}),
        json!({"context":"project","name":"On Startup","script":"SetupData.SetPackageProperties('component',{Install=false})"})
    ]);
    let mut file = checked_file("9.5");
    file["packages"] = json!(["component"]);
    assert_eq!(
        behavior::lower(&g, &[file], "9.5").unwrap()["file_conditions"],
        json!([])
    );
}

#[test]
fn global_functions_are_defined_once_before_install_events() {
    let mut g = graph("if eligible() then Screen.Next() end");
    g["events"].as_array_mut().unwrap().push(json!({"context":"project","name":"Global Functions","script":"function eligible() return true end"}));
    assert_eq!(
        behavior::lower(&g, &[checked_file("9.5")], "9.5").unwrap()["condition"],
        true
    );
}

#[test]
fn after_and_uninstall_screens_do_not_change_installation_choices_or_gates() {
    let mut g = graph("Screen.Next()");
    g["objects"] = json!([{"class":"CSUFPackage","name":"component","install":true}]);
    let mut file = checked_file("9.5");
    file["packages"] = json!(["component"]);
    for stage in ["after", "uninstall"] {
        let mut changed = g.clone();
        changed["events"].as_array_mut().unwrap().push(json!({"lifecycle":stage,
            "context":"CSUFScreenInstallFolder:same-name","name":"On Next",
            "script":"SetupData.SetPackageProperties('component',{Install=false}); if File.DoesExist('%AppFolder%\\\\post-only.dat') then Screen.Next() end"}));
        let result = behavior::lower(&changed, &[file.clone()], "9.5").unwrap();
        assert_eq!(result["condition"], true, "{stage}");
        assert_eq!(result["file_conditions"], json!([]), "{stage}");
    }
    g["events"]
        .as_array_mut()
        .unwrap()
        .push(json!({"lifecycle":"while",
        "context":"CSUFScreenProgressBars:progress","name":"On Preload",
        "script":"SetupData.SetPackageProperties('component',{Install=false})"}));
    assert_eq!(
        behavior::lower(&g, &[file], "9.5").unwrap()["file_conditions"],
        json!([{"path":"payload.bin","condition":false}])
    );
}
