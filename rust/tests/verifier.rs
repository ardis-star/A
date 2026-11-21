use ascension_asm::assemble;
use ascension_asm::bytecode::crc32c;
use ascension_asm::verifier::{decode_image, verify_image};

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn with_section(image: &[u8], kind: u32, flags: u32, content: &[u8]) -> Vec<u8> {
    let count = u32_at(image, 28) as usize;
    let insert_at = 40 + count * 32;
    let base = image.len() + 32;
    let new_off = (base + 7) & !7usize;
    let padding = new_off - base;
    let mut out = Vec::with_capacity(image.len() + 32 + padding + content.len());
    out.extend_from_slice(&image[..insert_at]);
    let mut entry = [0u8; 32];
    entry[0..4].copy_from_slice(&kind.to_le_bytes());
    entry[4..8].copy_from_slice(&flags.to_le_bytes());
    entry[8..16].copy_from_slice(&new_off.to_le_bytes());
    entry[16..24].copy_from_slice(&(content.len() as u64).to_le_bytes());
    entry[24..28].copy_from_slice(&8u32.to_le_bytes());
    entry[28..32].copy_from_slice(&crc32c(content).to_le_bytes());
    out.extend_from_slice(&entry);
    out.extend_from_slice(&image[insert_at..]);
    out.extend_from_slice(std::slice::from_ref(&0u8).repeat(padding).as_slice());
    out.extend_from_slice(content);
    out[28..32].copy_from_slice(&((count + 1) as u32).to_le_bytes());
    let total = out.len() as u64;
    out[12..20].copy_from_slice(&total.to_le_bytes());
    for i in 0..count {
        let e = 40 + i * 32;
        let off = u64_at(&out, e + 8) as usize;
        out[e + 8..e + 16].copy_from_slice(&((off + 32) as u64).to_le_bytes());
    }
    out
}

fn excp_record(function_id: u32, start: u32, end: u32, handler: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(16);
    b.extend_from_slice(&function_id.to_le_bytes());
    b.extend_from_slice(&start.to_le_bytes());
    b.extend_from_slice(&end.to_le_bytes());
    b.extend_from_slice(&handler.to_le_bytes());
    b
}

fn excp_section(records: &[Vec<u8>]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for r in records {
        b.extend_from_slice(r);
    }
    b
}

#[test]
fn verifier_valid_excp_section_accepted() {
    let source = "export main\nmain::\n    jump -> target\n    stop\ntarget::\n    return\n";
    let image = assemble(source).unwrap();
    let records = vec![excp_record(0, 0, 1, 1)];
    let image = with_section(&image, 7, 0, &excp_section(&records));
    assert!(verify_image(&image).is_ok());
}

#[test]
fn verifier_excp_handler_inside_protected_range_rejected() {
    let source = "export main\nmain::\n    jump -> target\n    stop\ntarget::\n    return\n";
    let image = assemble(source).unwrap();
    let records = vec![excp_record(0, 0, 1, 0)];
    let image = with_section(&image, 7, 0, &excp_section(&records));
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("inside its protected range")
    );
}

#[test]
fn verifier_excp_inverted_range_rejected() {
    let source = "export main\nmain::\n    jump -> target\n    stop\ntarget::\n    return\n";
    let image = assemble(source).unwrap();
    let records = vec![excp_record(0, 1, 0, 1)];
    let image = with_section(&image, 7, 0, &excp_section(&records));
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("empty or inverted")
    );
}

#[test]
fn verifier_excp_unmapped_block_rejected() {
    let source = "export main\nmain::\n    jump -> target\n    stop\ntarget::\n    return\n";
    let image = assemble(source).unwrap();
    let records = vec![excp_record(0, 99, 100, 1)];
    let image = with_section(&image, 7, 0, &excp_section(&records));
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("is not mapped")
    );
}

#[test]
fn verifier_excp_overlapping_ranges_rejected() {
    let source = "export main\nmain::\n    jump -> target\n    stop\ntarget::\n    return\n";
    let image = assemble(source).unwrap();
    let records = vec![excp_record(0, 0, 1, 1), excp_record(0, 0, 1, 1)];
    let image = with_section(&image, 7, 0, &excp_section(&records));
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("overlap")
    );
}

#[test]
fn verifier_excp_truncated_rejected() {
    let source = "export main\nmain::\n    return\n";
    let image = assemble(source).unwrap();
    let image = with_section(&image, 7, 0, &[1u8]);
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("truncated")
    );
}

#[test]
fn verifier_excp_length_mismatch_rejected() {
    let source = "export main\nmain::\n    return\n";
    let image = assemble(source).unwrap();
    let image = with_section(&image, 7, 0, &[5u8, 0, 0, 0]);
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("record count")
    );
}

#[test]
fn verifier_valid_data_section_accepted() {
    let source = "export main\nmain::\n    return\n";
    let image = assemble(source).unwrap();
    assert!(verify_image(&image).is_ok());
}

#[test]
fn verifier_v10_without_control_flow_accepted() {
    let source = "export main\nmain::\n    return\n";
    let image = assemble(source).unwrap();
    let decoded = decode_image(&image).unwrap();
    assert_eq!(decoded.minor_version, 0);
    assert!(verify_image(&image).is_ok());
}

#[test]
fn verifier_v10_with_blocks_section_rejected() {
    let source = "export main\nmain::\n    return\n";
    let image = assemble(source).unwrap();
    let image = with_section(&image, 9, 1, &[0u8, 0, 0, 0]);
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("minor version 1")
    );
}

#[test]
fn verifier_v11_with_control_flow_and_blocks_accepted() {
    let source = "export main\nmain::\n    jump -> target\n    stop\ntarget::\n    return\n";
    let image = assemble(source).unwrap();
    let decoded = decode_image(&image).unwrap();
    assert_eq!(decoded.minor_version, 1);
    assert!(verify_image(&image).is_ok());
}

#[test]
fn verifier_v11_without_control_flow_no_blocks_accepted() {
    let source = "export main\nmain::\n    return\n";
    let mut image = assemble(source).unwrap();
    image[6..8].copy_from_slice(&1u16.to_le_bytes());
    assert!(verify_image(&image).is_ok());
}

#[test]
fn verifier_unknown_major_version_rejected() {
    let mut image = assemble("export main\nmain::\n    return\n").unwrap();
    image[4..6].copy_from_slice(&2u16.to_le_bytes());
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("version")
    );
}

#[test]
fn verifier_unknown_flag_bit_rejected() {
    let source = "export main\nmain::\n    return\n";
    let mut image = assemble(source).unwrap();
    let section_index = u32_at(&image, 28) as usize - 1;
    let entry = 40 + section_index * 32;
    image[entry + 4..entry + 8].copy_from_slice(&0x8000_0000u32.to_le_bytes());
    let offset = u64_at(&image, entry + 8) as usize;
    let length = u64_at(&image, entry + 16) as usize;
    let crc = crc32c(&image[offset..offset + length]);
    image[entry + 28..entry + 32].copy_from_slice(&crc.to_le_bytes());
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("flags")
    );
}

#[test]
fn verifier_duplicate_section_rejected() {
    let source = "export main\nmain::\n    return\n";
    let image = assemble(source).unwrap();
    let kind = u32_at(&image, 40);
    let flags = u32_at(&image, 44);
    let offset = u64_at(&image, 48) as usize;
    let length = u64_at(&image, 56) as usize;
    let content = &image[offset..offset + length];
    let image = with_section(&image, kind, flags, content);
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}

#[test]
fn verifier_missing_required_section_rejected() {
    let source = "export main\nmain::\n    return\n";
    let mut image = assemble(source).unwrap();
    // Remove the DATA section entry by shrinking directory and shifting payloads is complex;
    // instead flip the CODE section kind to unknown so CODE (4) is missing.
    let entry = 40 + 2 * 32; // CODE is the third section in assembler output
    let kind = u32_at(&image, entry);
    assert_eq!(kind, 4);
    image[entry..entry + 4].copy_from_slice(&99u32.to_le_bytes());
    let offset = u64_at(&image, entry + 8) as usize;
    let length = u64_at(&image, entry + 16) as usize;
    let crc = crc32c(&image[offset..offset + length]);
    image[entry + 28..entry + 32].copy_from_slice(&crc.to_le_bytes());
    assert!(
        verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("required section")
    );
}
