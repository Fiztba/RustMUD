use mud_game::text::{file_to_string, load_help};

#[test]
fn text_files_preserve_long_lines_and_unterminated_final_bytes() {
    let path = std::env::temp_dir().join(format!("rustmud-text-lines-{}.txt", std::process::id()));
    for length in [1, 254, 255, 256, 700] {
        for ending in [&b""[..], b"\n", b"\r\n"] {
            let mut line = vec![b'x'; length]; line[length - 1] = b'Z';
            let mut input = line.clone(); input.extend_from_slice(ending);
            std::fs::write(&path, &input).unwrap();
            let mut expected = line; expected.extend_from_slice(b"\r\n");
            assert_eq!(file_to_string(&path).unwrap(), expected, "length={length}, ending={ending:?}");
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn help_lines_preserve_content_and_final_level_without_a_newline() {
    for ending in [&b"\n"[..], b"\r\n"] {
        let mut input = b"KEY".to_vec(); input.extend_from_slice(ending);
        input.extend_from_slice(&vec![b'x'; 700]); input.extend_from_slice(ending); input.extend_from_slice(b"#9");
        let mut log = Vec::new(); let help = load_help(&input, &mut log);
        assert_eq!(help.len(), 1); assert_eq!(help[0].min_level, 9);
        let mut expected = b"KEY\r\n".to_vec(); expected.extend_from_slice(&vec![b'x'; 700]); expected.extend_from_slice(b"\r\n");
        assert_eq!(help[0].entry.as_ref(), &expected); assert!(log.is_empty());
    }
}
