use ppl_engine::help::{Book, Language};
use sha2::{Digest, Sha256};

fn put(data: &mut [u8], at: usize, value: usize) {
    data[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
}
fn get(data: &[u8], at: usize) -> usize {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
}
fn compressed(units: &[u16]) -> Vec<u8> {
    // Synthetic four-leaf tree: 00=NUL, 01=A, 10=B, 11=isolated surrogate.
    let mut data = vec![0; ((units.len() + 1) * 2).div_ceil(8)];
    for (index, unit) in units.iter().chain([0].iter()).enumerate() {
        let code = match unit {
            0 => 0,
            65 => 2,
            66 => 1,
            0xd800 => 3,
            _ => panic!("unsupported test unit"),
        };
        data[index / 4] |= code << ((index % 4) * 2);
    }
    data
}
fn fixture() -> Vec<u8> {
    fixture_with_vacancy(false)
}
fn fixture_with_vacancy(vacant: bool) -> Vec<u8> {
    let dictionary = [0, 5, 0, 3, 0, 0, 65, 0, 3, 66, 0, 0, 216];
    let mut index = Vec::new();
    let mut texts = Vec::new();
    for id in 1..=2u32 {
        let reserved = vacant && id == 1;
        let ident = if reserved { 0 } else { id };
        let mut encoded = (ident as u16).to_le_bytes().to_vec();
        if !reserved {
            for units in [&[65u16, 66][..], &[0xd800, 65, 66][..], &[66][..]] {
                encoded.extend(compressed(units));
            }
        }
        index.extend(ident.to_le_bytes());
        index.extend((texts.len() as u32).to_le_bytes());
        index.extend((encoded.len() as u16).to_le_bytes());
        index.extend(if reserved { [128, 0] } else { [30, 2] });
        let name = if reserved {
            &[0u16][..]
        } else {
            &[65u16, 66, 0][..]
        };
        for unit in name {
            index.extend(unit.to_le_bytes());
        }
        texts.extend(encoded);
    }
    let mut group = compressed(&[65, 66]);
    group.push(17);
    group.extend(compressed(&[0xd800]));
    group.push(42);
    let mut book = vec![0; 332];
    for (at, value) in [
        (0, 0xb28a617c),
        (4, 0xfe84125d),
        (8, 332),
        (12, 1),
        (28, 2),
        (32, 332 + dictionary.len()),
        (36, index.len()),
        (40, 332),
        (44, dictionary.len()),
        (48, 332 + dictionary.len() + index.len()),
        (52, texts.len()),
        (56, 1),
        (60, 332 + dictionary.len() + index.len() + texts.len()),
        (64, 332 + dictionary.len() + index.len() + texts.len() + 12),
        (68, group.len()),
    ] {
        put(&mut book, at, value);
    }
    book.extend(dictionary);
    book.extend(index);
    book.extend(texts);
    for word in [(2u32 << 23) | 37, 0, group.len() as u32] {
        book.extend(word.to_le_bytes());
    }
    book.extend(group);
    let mut archive = vec![0; 28 + 128 * 8];
    for (index, value) in [u32::MAX as usize, 5, 1, 0, 3, 2, 6]
        .into_iter()
        .enumerate()
    {
        put(&mut archive, index * 4, value);
    }
    put(&mut archive, 28, 128 * 8);
    put(&mut archive, 28 + 72 * 8, 128 * 8);
    put(&mut archive, 28 + 72 * 8 + 4, book.len());
    archive.extend(book);
    archive
}

#[test]
fn an_original_vacant_index_is_explicit_and_cannot_alias_a_neighboring_topic() {
    let book = Book::from_resource(&fixture_with_vacancy(true), Language::English).unwrap();
    assert_eq!(book.topics[0].id, 0);
    assert!(book.topics[0].spans.is_empty());
    assert!(book.topic(0).is_none());
    assert!(book.topic(1).is_none());
    assert_eq!(book.topic(2).unwrap().name, "AB");
    assert_eq!(book.named("AB").count(), 1);
    assert_eq!(book.named("").count(), 0);
}

#[test]
fn raw_spans_byte_alignment_duplicate_names_and_group_string_indices_survive() {
    let book = Book::from_resource(&fixture(), Language::English).unwrap();
    assert_eq!(book.topics.len(), 2);
    assert_eq!(
        book.named("ab").map(|t| t.id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(book.topic(0).is_none());
    assert!(book.topic(3).is_none());
    for id in 1..=2 {
        let topic = book.topic(id).unwrap();
        assert_eq!(topic.mode, id as u16);
        assert_eq!(topic.kind, 30);
        assert_eq!(
            topic.spans,
            vec![vec![65, 66], vec![0xd800, 65, 66], vec![66]]
        );
    }
    assert_eq!(book.groups[0].id, 37);
    assert_eq!(book.string(37, 0), Some(&[65, 66][..]));
    assert_eq!(book.string(37, 1), Some(&[0xd800][..]));
    assert_eq!(book.string(37, 2), None);
    assert_eq!(book.string(0, 0), None);
}

#[test]
fn malformed_archives_fail_instead_of_exposing_a_partial_or_substitute_book() {
    let original = fixture();
    for length in 0..original.len() {
        assert!(
            Book::from_resource(&original[..length], Language::English).is_err(),
            "truncated at {length}"
        );
    }
    let start = 28 + 128 * 8;
    let text = start + get(&original, start + 48);
    let index = start + get(&original, start + 32);
    let cases = [
        (0, 0),
        (28 + 72 * 8, 0),
        (28 + 72 * 8 + 4, u32::MAX as usize),
        (start, 0),
        (start + 12, 2),
        (start + 28, 0),
        (start + 36, u32::MAX as usize),
        (index, 2),
    ];
    for (at, value) in cases {
        let mut bad = original.clone();
        put(&mut bad, at, value);
        assert!(
            Book::from_resource(&bad, Language::English).is_err(),
            "invalid word at {at}"
        );
    }
    for (at, value) in [
        (start + 332, 4),
        (start + 333, 255),
        (text, 3),
        (index + 16, 1),
    ] {
        let mut bad = original.clone();
        bad[at] = value;
        assert!(
            Book::from_resource(&bad, Language::English).is_err(),
            "invalid byte at {at}"
        );
    }
    let mut unterminated = original.clone();
    let len = u16::from_le_bytes(original[index + 8..index + 10].try_into().unwrap()) as usize;
    unterminated[text + 2..text + len].fill(255);
    assert!(Book::from_resource(&unterminated, Language::English).is_err());
    assert!(Book::from_resource(&original, Language::Japanese).is_err());
    // Mutations may form another valid book, but must never escape checked
    // decoding with a host panic. Include every byte of the compact fixture.
    for at in 0..original.len() {
        let mut bad = original.clone();
        bad[at] ^= 255;
        let _ = Book::from_resource(&bad, Language::English);
    }
}

fn hash_topic(topic: &ppl_engine::help::Topic, digest: &mut Sha256) {
    digest.update(topic.id.to_le_bytes());
    digest.update([topic.kind]);
    digest.update(topic.mode.to_le_bytes());
    let name = topic.name.encode_utf16().collect::<Vec<_>>();
    digest.update((name.len() as u32).to_le_bytes());
    for unit in name {
        digest.update(unit.to_le_bytes());
    }
    digest.update((topic.spans.len() as u32).to_le_bytes());
    for span in &topic.spans {
        digest.update((span.len() as u32).to_le_bytes());
        for unit in span {
            digest.update(unit.to_le_bytes());
        }
    }
}

#[test]
fn installed_books_match_independent_decoder_for_every_topic_in_all_languages() {
    let Some(resource) = std::env::var_os("PRIME_TEST_FONT_RESOURCE") else {
        eprintln!("Installed Help validation requires PRIME_TEST_FONT_RESOURCE");
        return;
    };
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-help-resource-controls.json"
    ))
    .unwrap();
    let data = std::fs::read(&resource).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&data)),
        fixture["archive_sha256"]
    );
    for language in Language::ALL {
        let expected = fixture["books"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["language"] == language as u32)
            .unwrap();
        let book = Book::from_resource(&data, language).unwrap();
        assert_eq!(
            book.topics.len() as u64,
            expected["topics"].as_u64().unwrap()
        );
        assert_eq!(
            book.groups.len() as u64,
            expected["groups"].as_u64().unwrap()
        );
        let mut digest = Sha256::new();
        let mut units = 0;
        let mut spans = 0;
        for topic in &book.topics {
            hash_topic(topic, &mut digest);
            spans += topic.spans.len();
            units += topic.spans.iter().map(Vec::len).sum::<usize>();
        }
        assert_eq!(
            format!("{:x}", digest.finalize()),
            expected["decoded_sha256"],
            "{language:?}"
        );
        assert_eq!(units as u64, expected["text_units"].as_u64().unwrap());
        assert_eq!(spans as u64, expected["spans"].as_u64().unwrap());
        let mut digest = Sha256::new();
        let mut strings = 0;
        let mut group_units = 0;
        for group in &book.groups {
            digest.update(group.id.to_le_bytes());
            digest.update((group.strings.len() as u32).to_le_bytes());
            strings += group.strings.len();
            for text in &group.strings {
                digest.update((text.len() as u32).to_le_bytes());
                group_units += text.len();
                for unit in text {
                    digest.update(unit.to_le_bytes());
                }
            }
        }
        assert_eq!(
            format!("{:x}", digest.finalize()),
            expected["groups_sha256"],
            "{language:?}"
        );
        assert_eq!(strings as u64, expected["group_strings"].as_u64().unwrap());
        assert_eq!(
            group_units as u64,
            expected["group_text_units"].as_u64().unwrap()
        );
        for selected in expected["selected"].as_array().unwrap() {
            let topic = book.topic(selected["id"].as_u64().unwrap() as u32).unwrap();
            assert_eq!(topic.name, selected["name"]);
            let mut digest = Sha256::new();
            hash_topic(topic, &mut digest);
            assert_eq!(format!("{:x}", digest.finalize()), selected["sha256"]);
        }
    }
}
