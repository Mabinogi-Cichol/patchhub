use serde_json::json;
use steam_patch_installer_format::audit::*;

fn crc(data: &[u8]) -> u32 {
    let mut n = !0u32;
    for b in data {
        n ^= *b as u32;
        for _ in 0..8 {
            n = if n & 1 == 1 {
                (n >> 1) ^ 0xedb88320
            } else {
                n >> 1
            };
        }
    }
    !n
}
fn string(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    assert!(bytes.len() < 255);
    let mut out = vec![bytes.len() as u8];
    out.extend(bytes);
    out
}
fn packed(bytes: &[u8]) -> Vec<u8> {
    let mut result = vec![16];
    result.extend((bytes.len() as u64).to_le_bytes());
    for chunk in bytes.chunks(65536) {
        result.push(if result.len() == 9 { 1 } else { 2 });
        result.extend(((chunk.len() - 1) as u16).to_be_bytes());
        result.extend(chunk);
    }
    result.push(0);
    result
}
fn record(name: &str, directory: &str, data: &[u8], version: &str) -> (Vec<u8>, Vec<u8>) {
    record_declaring_packages(
        name,
        directory,
        data,
        version,
        1,
        &[String::from("expansion")],
    )
}
fn record_declaring_packages(
    name: &str,
    directory: &str,
    data: &[u8],
    version: &str,
    declared_packages: u16,
    packages: &[String],
) -> (Vec<u8>, Vec<u8>) {
    let payload = packed(data);
    let mut out = Vec::new();
    for s in [
        format!("C:\\source\\{name}"),
        name.into(),
        "C:\\source".into(),
        ".txt".into(),
        "Archive".into(),
        "".into(),
    ] {
        out.extend(string(&s));
    }
    out.extend([0; 2]);
    out.extend((data.len() as i64).to_le_bytes());
    out.extend([0; 5]);
    out.extend([0; 8]);
    out.extend([0; 16]);
    out.extend([0; 8]);
    out.extend([0; 25]);
    out.extend(string(directory));
    out.extend([0; 11]);
    for _ in 0..5 {
        out.extend(string(""));
    }
    out.push(0);
    out.extend(string(""));
    out.extend([0; 8]);
    out.extend(string(""));
    out.extend(vec![0; if version == "9.7" { 4 } else { 3 }]);
    out.extend([1, 1, 0]);
    out.extend([0; 10]);
    out.extend(0u16.to_le_bytes());
    out.extend(string("NeedsDLC()"));
    out.extend([0; 2]);
    out.extend(string(""));
    out.extend(string(""));
    out.extend(declared_packages.to_le_bytes());
    for value in packages {
        out.extend(string(value));
    }
    out.extend(string("notes"));
    out.extend((payload.len() as i64).to_le_bytes());
    out.extend(crc(data).to_le_bytes());
    out.extend([0; 8]);
    (out, payload)
}
fn metadata(records: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0, 0];
    out.extend(b"unparsed-events");
    out.extend((records.len() as u16).to_le_bytes());
    out.extend([0xff, 0xff, 1, 0]);
    out.extend(14u16.to_le_bytes());
    out.extend(b"CSetupFileData");
    out.extend([0; 5]);
    for r in records {
        out.extend(&r.0);
    }
    out.extend(b"unparsed-after-table");
    out
}
fn overlay(records: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = SIGNATURE.to_vec();
    out.extend([0; 10]);
    for bytes in [b"not-an-executable".as_slice(), b"not-a-dll".as_slice()] {
        out.extend((bytes.len() as i64).to_le_bytes());
        out.extend(bytes);
    }
    out.extend(3u32.to_le_bytes());
    let md = metadata(records);
    for (name, bytes) in [
        ("irsetup.dat", md.as_slice()),
        ("IRIMG1.BMP", b"image1"),
        ("IRIMG2.BMP", b"image2"),
    ] {
        let payload = packed(bytes);
        let mut name_raw = [0; 264];
        name_raw[..name.len()].copy_from_slice(name.as_bytes());
        out.extend(name_raw);
        out.extend((payload.len() as i64).to_le_bytes());
        out.extend(crc(bytes).to_le_bytes());
        out.extend([0; 4]);
        out.extend(payload);
    }
    for r in records {
        out.extend(&r.1);
    }
    out
}
fn put16(data: &mut [u8], at: usize, n: u16) {
    data[at..at + 2].copy_from_slice(&n.to_le_bytes());
}
fn put32(data: &mut [u8], at: usize, n: u32) {
    data[at..at + 4].copy_from_slice(&n.to_le_bytes());
}
fn manifest_resource(manifest: &[u8]) -> Vec<u8> {
    let resource_size = 1024usize.max(128 + manifest.len());
    let mut resource = vec![0; resource_size];
    for (at, id, child) in [(0, 24, 0x80000020), (32, 1, 0x80000040), (64, 1033, 96)] {
        put16(&mut resource, at + 14, 1);
        put32(&mut resource, at + 16, id);
        put32(&mut resource, at + 20, child);
    }
    put32(&mut resource, 96, 0x1080);
    put32(&mut resource, 100, manifest.len() as u32);
    resource[128..128 + manifest.len()].copy_from_slice(manifest);
    resource
}
fn container(records: &[(Vec<u8>, Vec<u8>)], resource: &[u8]) -> Vec<u8> {
    let resource_size = resource.len() as u32;
    let mut headers = vec![0; 512];
    headers[..2].copy_from_slice(b"MZ");
    put32(&mut headers, 60, 64);
    headers[64..68].copy_from_slice(b"PE\0\0");
    put16(&mut headers, 70, 1);
    put16(&mut headers, 84, 224);
    put16(&mut headers, 88, 0x10b);
    put32(&mut headers, 148, 512);
    put32(&mut headers, 180, 16);
    put32(&mut headers, 200, 0x1000);
    put32(&mut headers, 204, resource_size);
    for (at, n) in [
        (320, resource_size),
        (324, 0x1000),
        (328, resource_size),
        (332, 512),
    ] {
        put32(&mut headers, at, n);
    }
    let mut out = headers;
    out.extend_from_slice(resource);
    out.extend(overlay(records));
    out
}
fn sample(
    records: &[(Vec<u8>, Vec<u8>)],
    version: &str,
    certificate: bool,
    xml: Option<&str>,
) -> Vec<u8> {
    let default=format!("<assembly><assemblyIdentity version=\"{version}.1.0\"/><description>Setup Factory Run-time</description><dependency><assemblyIdentity version=\"6.0.0.0\"/></dependency></assembly>");
    let manifest = xml.unwrap_or(&default).as_bytes();
    let mut out = container(records, &manifest_resource(manifest));
    if certificate {
        while !out.len().is_multiple_of(8) {
            out.push(0);
        }
        let total = out.len() as u32;
        put32(&mut out, 216, total);
        put32(&mut out, 220, 16);
        out.extend(12u32.to_le_bytes());
        out.extend(0x200u16.to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(b"cert");
        out.extend([0; 4]);
    }
    out
}
fn rejects<T>(result: steam_patch_installer_format::Result<T>, code: &str) {
    assert_eq!(result.err().unwrap().0, code);
}

#[test]
fn mfc_strings_are_bounded_and_strict() {
    let text = "x".repeat(300);
    let mut bytes = vec![255];
    bytes.extend(300u16.to_le_bytes());
    bytes.extend(text.as_bytes());
    assert_eq!(Reader::new(&bytes).string().unwrap(), text);
    let mut bytes = vec![255, 255, 255];
    bytes.extend(300u32.to_le_bytes());
    bytes.extend(text.as_bytes());
    assert_eq!(Reader::new(&bytes).string().unwrap(), text);
    let mut bytes = vec![255, 254, 255, 2];
    bytes.extend("中文".encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(Reader::new(&bytes).string().unwrap(), "中文");
    rejects(Reader::new(&[3, b'a', 0, b'b']).string(), "embedded_nul");
    rejects(Reader::new(&[1, 255]).string(), "invalid_utf8");
    rejects(Reader::new(&[4, b'x']).string(), "truncated_record");
    let mut bytes = vec![255, 255, 255];
    bytes.extend((MAX_METADATA as u32 + 1).to_le_bytes());
    rejects(Reader::new(&bytes).string(), "string_size_limit");
    rejects(Reader::at(b"x", 2, 1), "invalid_range");
}
#[test]
fn destination_paths_and_unicode_conflicts() {
    assert_eq!(
        safe_target("%AppFolder%", "extra.xp3").unwrap(),
        "extra.xp3"
    );
    assert_eq!(
        safe_target("%AppFolder%\\www", "测试.txt").unwrap(),
        "www/测试.txt"
    );
    for path in [
        "%AppFolder%\\..",
        "%AppFolder%\\C:",
        "%AppFolder%\\www\\NUL",
        "%AppFolder%\\www\\COM¹",
        "%AppFolder%\\www\\x ",
        "%AppFolder%\\www\\.steam-patch-backup",
    ] {
        assert!(safe_target(path, "x").is_err());
    }
    for name in [
        "a:b", "../x", "..\\x", "a\0b", "C:\\x", "foo/xx", "foo\\xx", "LPT9.txt", "x.",
    ] {
        assert!(safe_target("%AppFolder%\\www", name).is_err());
    }
    rejects(
        safe_target("%Temp%\\www", "x"),
        "unsupported_target_variable",
    );
    for (names, code) in [
        (["A.txt", "a.txt"], "duplicate_or_unicode_case_collision"),
        (
            ["é.txt", "e\u{301}.txt"],
            "duplicate_or_unicode_case_collision",
        ),
        (["www", "www/data"], "file_directory_collision"),
        (["A/x", "a/y"], "duplicate_or_unicode_case_collision"),
        (["Straße", "STRASSE"], "duplicate_or_unicode_case_collision"),
    ] {
        rejects(
            validate_targets(&names.map(|name| json!({"destination":name}))),
            code,
        );
    }
}
#[test]
fn crc_boundary_preserves_framing_and_tail() {
    let mut a = record("a", "%AppFolder%", b"first", "9.5");
    let mut b = record("b", "%AppFolder%", b"second", "9.5");
    let a_size = a.0.len() - 8;
    let framing = [1, 4, 128, 1, 0, 0, 0, 0];
    a.0[a_size..].copy_from_slice(&framing);
    let footer = [0, 0, 0, 4, 0, 255, 255, 1];
    let b_size = b.0.len() - 8;
    b.0[b_size..].copy_from_slice(&footer);
    let raw = metadata(&[a, b]);
    let (rows, table) = file_table(&raw, "9.5").unwrap();
    assert_eq!(rows[0]["serialized_record"]["size"], a_size);
    assert_eq!(
        rows[0]["unparsed_inter_record_framing"]["base64"],
        "AQSAAQAAAAA="
    );
    assert!(rows[1]["unparsed_inter_record_framing"].is_null());
    assert_eq!(rows[0]["unreviewed_fields"].as_array().unwrap().len(), 11);
    let end = table["end"].as_u64().unwrap() as usize;
    assert_eq!(&raw[end..end + 8], footer);
    assert_eq!(
        table["end_basis"],
        "last_known_crc_end_not_serialized_object_end"
    );
}
#[test]
fn full_container_all_versions_and_certificate() {
    for version in ["9.5", "9.6", "9.7"] {
        let raw = sample(
            &[
                record("测试.txt", "%AppFolder%\\www", b"test-data", version),
                record("root.xp3", "%AppFolder%", b"other", version),
            ],
            version,
            true,
            None,
        );
        let report = audit(&raw, false).unwrap();
        assert_eq!(report["installer"], format!("Setup Factory {version}"));
        assert_eq!(report["files"][0]["destination"], "www/测试.txt");
        assert_eq!(report["files"][0]["condition"], "NeedsDLC()");
        assert_eq!(report["files"][0]["packages"], json!(["expansion"]));
        assert_eq!(
            report["files"][0]["payload"]["crc32"],
            report["files"][0]["crc32"]
        );
        assert_eq!(report["files"][1]["payload"]["size"], 5);
        assert_eq!(report["capability"], "discover_only");
        assert_eq!(report["installable"], false);
        assert_eq!(report["certificate"]["signature_verified"], false);
        assert!(report["script_inventory"].is_object());
    }
}
#[test]
fn pe_resource_xml_certificate_and_tail_rejections() {
    let rows = [record("x", "%AppFolder%", b"hello", "9.5")];
    rejects(pe_overlay(b"html"), "not_pe");
    let mut bytes = vec![0; 102];
    bytes[..2].copy_from_slice(b"MZ");
    rejects(pe_overlay(&bytes), "not_pe");
    rejects(
        pe_overlay(&sample(&rows, "10.0", false, None)),
        "unsupported_installer_version",
    );
    rejects(
        audit(&sample(&rows, "9.5", false, None)[..1000], false),
        "truncated_pe_section",
    );
    let mut bytes = sample(&rows, "9.5", false, None);
    put32(&mut bytes, 332, 256);
    rejects(pe_overlay(&bytes), "overlapping_pe_headers");
    let mut bytes = sample(&rows, "9.5", false, None);
    put32(&mut bytes, 532, 0x8000ffff);
    rejects(pe_overlay(&bytes), "invalid_range");
    let mut bytes = sample(&rows, "9.5", true, None);
    *bytes.last_mut().unwrap() = 1;
    rejects(audit(&bytes, false), "invalid_certificate_padding");
    let mut bytes = sample(&rows, "9.5", false, None);
    bytes.push(1);
    rejects(audit(&bytes, false), "overlay_size_or_tail_mismatch");
    for xml in ["<!DOCTYPE assembly [<!ENTITY x '9.5.1.0'>]><assembly/>","<assembly><assemblyIdentity version='9.5.1.0'/><description>Setup Factory Run-time</description></assembly><second/>"]{assert!(pe_overlay(&sample(&rows,"9.5",false,Some(xml))).is_err());}
    let xml="<a:assembly xmlns:a='urn:x'><a:assemblyIdentity version='9.5.1.0'/><a:description>Setup Factory Run-time</a:description></a:assembly>";
    assert!(pe_overlay(&sample(&rows, "9.5", false, Some(xml))).is_ok());
}
#[test]
fn decoder_stream_crc_length_eof_trailing_and_dictionary_caps() {
    let payload = packed(b"hello");
    let mut sink = Vec::new();
    let info = decode_payload(
        &payload,
        0,
        payload.len() as u64,
        5,
        crc(b"hello"),
        true,
        Some(&mut sink),
    )
    .unwrap();
    assert_eq!(sink, b"hello");
    assert_eq!(info["size"], 5);
    rejects(
        decode_payload(&payload, 0, payload.len() as u64, 5, 99, true, None),
        "payload_crc_mismatch",
    );
    rejects(
        decode_payload(&payload, 0, payload.len() as u64, 4, 0, true, None),
        "lzma2_size_mismatch",
    );
    let truncated = &payload[..payload.len() - 1];
    rejects(
        decode_payload(
            truncated,
            0,
            truncated.len() as u64,
            5,
            crc(b"hello"),
            true,
            None,
        ),
        "truncated_lzma2_stream",
    );
    let mut bytes = payload.clone();
    bytes.push(1);
    rejects(
        decode_payload(&bytes, 0, bytes.len() as u64, 5, crc(b"hello"), true, None),
        "trailing_compressed_data",
    );
    let mut bytes = payload.clone();
    bytes[0] = 40;
    rejects(
        decode_payload(&bytes, 0, bytes.len() as u64, 5, 0, true, None),
        "dictionary_limit",
    );
    bytes[0] = 41;
    rejects(
        decode_payload(&bytes, 0, bytes.len() as u64, 5, 0, true, None),
        "unsupported_compression",
    );
    let mut bytes = payload.clone();
    bytes[1..9].copy_from_slice(&4u64.to_le_bytes());
    rejects(
        decode_payload(&bytes, 0, bytes.len() as u64, 4, 0, true, None),
        "payload_size_mismatch",
    );
    rejects(
        decode_payload(b"hello", 0, 5, 4, 0, false, None),
        "payload_size_mismatch",
    );
    rejects(
        decode_payload(b"hello", 0, 6, 5, 0, false, None),
        "truncated_payload",
    );
    let bytes = vec![b'x'; CHUNK_TEST * 3];
    let payload = packed(&bytes);
    let mut sink = Vec::new();
    decode_payload(
        &payload,
        0,
        payload.len() as u64,
        bytes.len() as u64,
        crc(&bytes),
        true,
        Some(&mut sink),
    )
    .unwrap();
    assert_eq!(sink, bytes);
}
const CHUNK_TEST: usize = 65536;
#[test]
fn table_limits_marker_and_duplicate_destinations() {
    let mut bytes = metadata(&[record("x", "%AppFolder%", b"x", "9.5")]);
    let offset = bytes
        .windows(14)
        .position(|w| w == b"CSetupFileData")
        .unwrap()
        - 8;
    // A u16 count of 8193 is within the real SF field width; the table is then
    // bounded by the actual serialized record bytes, so this is truncation.
    put16(&mut bytes, offset, 8193);
    rejects(file_table(&bytes, "9.5"), "truncated_record");
    // Zero remains a genuine "no entries" rejection.
    let mut empty = metadata(&[record("x", "%AppFolder%", b"x", "9.5")]);
    put16(&mut empty, offset, 0);
    rejects(file_table(&empty, "9.5"), "file_count_limit");
    rejects(file_table(b"nothing", "9.5"), "ambiguous_file_table");
    let mut bytes = metadata(&[record("x", "%AppFolder%", b"x", "9.5")]);
    bytes.extend(b"CSetupFileData");
    rejects(file_table(&bytes, "9.5"), "ambiguous_file_table");
    let raw = sample(
        &[
            record("A.txt", "%AppFolder%", b"first", "9.5"),
            record("a.txt", "%AppFolder%", b"second", "9.5"),
        ],
        "9.5",
        false,
        None,
    );
    rejects(audit(&raw, false), "duplicate_or_unicode_case_collision");
}

#[test]
fn xml_entities_attributes_namespaces_and_character_rules() {
    let rows = [record("x", "%AppFolder%", b"hello", "9.5")];
    let valid="<a:assembly xmlns:a='urn:test' extra='&amp;'><a:assemblyIdentity version='9.&#53;.1.0'/><a:description>Setup Factory <!--comment-->&#82;un-time</a:description></a:assembly>";
    let (_, _, manifest, _) = pe_overlay(&sample(&rows, "9.5", false, Some(valid))).unwrap();
    assert_eq!(manifest["version"], "9.5.1.0");
    for root in [
        "<assembly extra='&unknown;'>",
        "<assembly extra='a' extra='b'>",
        "<unbound:assembly>",
        "<assembly extra='&#0;'>",
        "<assembly extra='\0'>",
        "<assembly xmlns:a='urn:x' a:1bad='value'>",
    ] {
        let close = if root.starts_with("<unbound:") {
            "</unbound:assembly>"
        } else {
            "</assembly>"
        };
        let xml=format!("{root}<assemblyIdentity version='9.5.1.0'/><description>Setup Factory Run-time</description>{close}");
        rejects(
            pe_overlay(&sample(&rows, "9.5", false, Some(&xml))),
            "invalid_manifest_xml",
        );
    }
}

#[test]
fn behavior_digest_matches_ascii_sorted_python_encoding() {
    use sha2::{Digest, Sha256};
    let value = json!({"z":[true,null,42],"a":"中😀\u{7f}\n"});
    let expected = br#"{"a":"\u4e2d\ud83d\ude00\u007f\n","z":[true,null,42]}"#;
    assert_eq!(
        behavior_digest(&value).unwrap(),
        format!("{:x}", Sha256::digest(expected))
    );
}

#[test]
fn xml_expanded_attribute_names_and_namespace_depth_are_bounded() {
    let rows = [record("x", "%AppFolder%", b"hello", "9.5")];
    let children =
        "<assemblyIdentity version='9.5.1.0'/><description>Setup Factory Run-time</description>";
    let duplicate=format!("<assembly xmlns:a='urn:test' xmlns:b='urn:test' a:key='one' b:key='two'>{children}</assembly>");
    rejects(
        pe_overlay(&sample(&rows, "9.5", false, Some(&duplicate))),
        "invalid_manifest_xml",
    );
    let distinct=format!("<assembly xmlns:a='urn:one' xmlns:b='urn:two' a:key='one' b:key='two'>{children}</assembly>");
    assert!(pe_overlay(&sample(&rows, "9.5", false, Some(&distinct))).is_ok());
    for (nested, accepted) in [(95, true), (96, false)] {
        let xml = format!(
            "<assembly>{}{}{children}</assembly>",
            "<x>".repeat(nested),
            "</x>".repeat(nested)
        );
        let result = pe_overlay(&sample(&rows, "9.5", false, Some(&xml)));
        if accepted {
            assert!(result.is_ok());
        } else {
            rejects(result, "manifest_depth_limit");
        }
    }
}

#[test]
fn declared_package_and_resource_counts_are_read_in_full() {
    // The SF file table stores the package reference count as a genuine u16; the
    // record's own serialized bytes bound how many references actually exist.
    let packages: Vec<String> = (0..300).map(|index| format!("pkg{index}")).collect();
    let (bytes, payload) =
        record_declaring_packages("x", "%AppFolder%", b"x", "9.5", 300, &packages);
    let (rows, _) = file_table(&metadata(&[(bytes, payload)]), "9.5").unwrap();
    assert_eq!(rows[0]["packages"].as_array().unwrap().len(), 300);
    assert_eq!(rows[0]["packages"][299], json!("pkg299"));
    // A reference list the record's bytes cannot fill is still truncation. Keep
    // the declared count and the first partial reference, drop the fields after
    // the list so only the actual buffer bounds the read.
    let (mut bytes, payload) =
        record_declaring_packages("x", "%AppFolder%", b"x", "9.5", 300, &packages);
    let block: usize = packages.iter().map(|package| package.len() + 1).sum();
    bytes.truncate(bytes.len() - (1 + 8 + 4 + 8) - block + 3);
    rejects(
        file_table(&metadata(&[(bytes, payload)]), "9.5"),
        "truncated_record",
    );
}

fn wide_resource(manifest: &[u8], entries: usize) -> Vec<u8> {
    let name_dir = 16 + entries * 8;
    let language_dir = name_dir + 24;
    let leaf = language_dir + 24;
    let manifest_at = leaf + 16;
    let mut resource = vec![0; manifest_at + manifest.len()];
    put16(&mut resource, 14, entries as u16);
    for index in 0..entries {
        let at = 16 + index * 8;
        if index + 1 == entries {
            put32(&mut resource, at, 24);
            put32(&mut resource, at + 4, 0x8000_0000 | name_dir as u32);
        } else {
            put32(&mut resource, at, 1000 + index as u32);
        }
    }
    put16(&mut resource, name_dir + 14, 1);
    put32(&mut resource, name_dir + 16, 1);
    put32(
        &mut resource,
        name_dir + 20,
        0x8000_0000 | language_dir as u32,
    );
    put16(&mut resource, language_dir + 14, 1);
    put32(&mut resource, language_dir + 16, 1033);
    put32(&mut resource, language_dir + 20, leaf as u32);
    put32(&mut resource, leaf, 0x1000 + manifest_at as u32);
    put32(&mut resource, leaf + 4, manifest.len() as u32);
    resource[manifest_at..manifest_at + manifest.len()].copy_from_slice(manifest);
    resource
}

#[test]
fn resource_directory_beyond_256_entries_still_finds_manifest() {
    let rows = [record("x", "%AppFolder%", b"hello", "9.5")];
    let manifest=b"<assembly><assemblyIdentity version=\"9.5.1.0\"/><description>Setup Factory Run-time</description></assembly>";
    for entries in [257usize, 300] {
        let raw = container(&rows, &wide_resource(manifest, entries));
        let (_, _, info, _) = pe_overlay(&raw).unwrap();
        assert_eq!(info["version"], "9.5.1.0");
    }
    // The declared directory count is still bounded by the actual resource area.
    let mut resource = wide_resource(manifest, 257);
    put16(&mut resource, 14, u16::MAX);
    rejects(pe_overlay(&container(&rows, &resource)), "truncated_record");
}

#[test]
fn namespace_uris_are_interned_once_per_binding_not_per_attribute() {
    let rows = [record("x", "%AppFolder%", b"hello", "9.5")];
    // This is <1 MiB XML, but copying its URI for each attribute would request
    // about 5 GiB. The parser retains one borrowed URI and small integer/local keys.
    let uri = format!("urn:{}", "n".repeat(512 * 1024));
    let mut xml = format!("<assembly xmlns:p='{uri}'");
    for index in 0..10_000 {
        xml.push_str(&format!(" p:a{index}=''"));
    }
    xml.push_str("><assemblyIdentity version='9.5.1.0'/><description>Setup Factory Run-time</description></assembly>");
    assert!(xml.len() < 1024 * 1024);
    assert!(pe_overlay(&sample(&rows, "9.5", false, Some(&xml))).is_ok());
    let aliases="<assembly xmlns:a='urn:one' xmlns:b='urn:&#111;ne' a:key='one' b:key='two'><assemblyIdentity version='9.5.1.0'/><description>Setup Factory Run-time</description></assembly>";
    rejects(
        pe_overlay(&sample(&rows, "9.5", false, Some(aliases))),
        "invalid_manifest_xml",
    );
}
