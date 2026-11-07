use patchhub_installer_format::archive::*;

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
fn rejects<T>(result: patchhub_installer_format::Result<T>, code: &str) {
    assert_eq!(result.err().unwrap().0, code);
}

#[test]
fn string_encodings_and_unconsumed_text_framing() {
    assert_eq!(
        Reader::new(&[4, 0xc4, 0xda, 0xc8, 0xdd]).string().unwrap(),
        "内容"
    );
    assert_eq!(Reader::new(&string("测试")).string().unwrap(), "测试");
    let mut wide = vec![255, 254, 255, 2];
    wide.extend("中文".encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(Reader::new(&wide).string().unwrap(), "中文");
    assert!(Reader::new(&[1, 0xff]).string().is_err());
    assert!(Reader::new(&[1, 0xff]).skip_string().is_ok());
    rejects(Reader::new(&[4, b'x']).string(), "truncated_record");
}
#[test]
fn all_versions_extract_all_payloads_including_duplicate_targets() {
    for version in ["9.5", "9.6", "9.7"] {
        let raw = sample(
            &[
                record("重复.txt", "%AppFolder%\\www", b"first", version),
                record("重复.txt", "%AppFolder%\\www", b"second", version),
            ],
            version,
            true,
            None,
        );
        let archive = read(&raw).unwrap();
        assert_eq!(archive.members().len(), 2);
        assert_eq!(archive.members()[0].path, "www/重复.txt");
        for (index, expected) in [b"first".as_slice(), b"second".as_slice()]
            .into_iter()
            .enumerate()
        {
            let mut sink = Vec::new();
            let info = archive.extract(&raw, index, &mut sink).unwrap();
            assert_eq!(sink, expected);
            assert_eq!(info.size, expected.len() as u64);
        }
    }
}
#[test]
fn scripts_components_conditions_and_unused_invalid_text_do_not_gate_members() {
    let (mut rec, payload) = record("x", "%AppFolder%", b"hello", "9.6");
    rec[1] = 0xff;
    let condition = rec.windows(10).position(|w| w == b"NeedsDLC()").unwrap();
    rec[condition..condition + 10].fill(0xff);
    let raw = sample(&[(rec, payload)], "9.6", false, None);
    let archive = read(&raw).unwrap();
    let mut sink = Vec::new();
    archive.extract(&raw, 0, &mut sink).unwrap();
    assert_eq!(sink, b"hello");
}
#[test]
fn gbk_destination_basename_is_preserved() {
    let (mut rec, payload) = record("NAME", "%AppFolder%\\www", b"hello", "9.6");
    let pos = rec.windows(5).position(|w| w == b"\x04NAME").unwrap();
    rec[pos + 1..pos + 5].copy_from_slice(&[0xc4, 0xda, 0xc8, 0xdd]);
    let raw = sample(&[(rec, payload)], "9.6", false, None);
    assert_eq!(read(&raw).unwrap().members()[0].path, "www/内容");
}
#[test]
fn decoder_checks_size_crc_and_actual_truncation() {
    let mut rows = [record("x", "%AppFolder%", b"hello", "9.6")];
    let raw = sample(&rows, "9.6", false, None);
    let archive = read(&raw).unwrap();
    let mut damaged = raw.clone();
    *damaged.last_mut().unwrap() = 1;
    assert!(archive.extract(&damaged, 0, &mut Vec::new()).is_err());
    let n = rows[0].0.len();
    rows[0].0[n - 12..n - 8].copy_from_slice(&0u32.to_le_bytes());
    let raw = sample(&rows, "9.6", false, None);
    let archive = read(&raw).unwrap();
    rejects(
        archive.extract(&raw, 0, &mut Vec::new()),
        "payload_crc_mismatch",
    );
    assert!(read(&raw[..raw.len() - 1]).is_err());
}
#[test]
fn relative_paths_keep_literal_components_and_protect_root_boundary() {
    assert_eq!(relative_path("%AppFolder%\\www", "x").unwrap(), "www/x");
    assert_eq!(
        relative_path("docs", "Readme.txt").unwrap(),
        "docs/Readme.txt"
    );
    assert_eq!(
        relative_path("%OtherFolder%", "x").unwrap(),
        "%OtherFolder%/x"
    );
    for directory in [
        "../outside",
        "/outside",
        "C:\\outside",
        "\\\\server\\outside",
    ] {
        assert!(relative_path(directory, "x").is_err());
    }
}
