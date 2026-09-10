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

#[test]
fn empty_lines_color_markers_and_size_limits_keep_their_meaning() {
    let path = std::env::temp_dir().join(format!("rustmud-text-edge-{}.txt", std::process::id()));
    for (input, expected) in [
        (&b""[..], &b""[..]), (b"\n", b"\r\n"), (b"\r\n\n", b"\r\n\r\n"),
        (b"first\r\n\r\nlast", b"first\r\n\r\nlast\r\n"),
        (b"@Rred@@literal\n", b"@Rred@@literal\r\n"),
        (b"a\rb\n", b"a\rb\r\n"), (b"last\r", b"last\r\r\n"),
    ] {
        std::fs::write(&path, input).unwrap(); assert_eq!(file_to_string(&path).unwrap(), expected);
    }
    let max = mud_data::types::MAX_STRING_LENGTH;
    for length in [max - 3, max - 2, max + 100] {
        std::fs::write(&path, vec![b'x'; length]).unwrap(); let result = file_to_string(&path).unwrap();
        assert_eq!(result.len(), if length == max - 3 { max - 1 } else { 0 });
    }
    std::fs::remove_file(&path).unwrap(); assert!(file_to_string(&path).is_none());
    let mut log = Vec::new(); let help = load_help(b"ONE TWO\r\nfirst\r\n\r\n@Rlast@@\r\n#9\r\n$~", &mut log);
    assert_eq!(help.len(), 2); assert_eq!(help[0].keyword, b"one"); assert_eq!(help[1].keyword, b"two");
    assert!(std::rc::Rc::ptr_eq(&help[0].entry, &help[1].entry));
    assert_eq!(help[0].entry.as_ref(), b"ONE TWO\r\nfirst\r\n\r\n\tRlast@@\r\n");
    assert_eq!(help[0].min_level, 9); assert!(log.is_empty());
}
