use read_fonts::{FileRef, FontRef, TableProvider};

/// Select the first face carrying the Windows UI family name used by the app.
///
/// Invalid font data deliberately follows the existing fallback behavior: use
/// face zero and let egui handle any later loading failure.
pub fn windows_ui_face_index(bytes: &[u8]) -> u32 {
    let count = match FileRef::new(bytes) {
        Ok(FileRef::Collection(collection)) => collection.len(),
        Ok(FileRef::Font(_)) | Err(_) => 1,
    };

    (0..count)
        .find(|&index| {
            FontRef::from_index(bytes, index)
                .ok()
                .is_some_and(has_windows_ui_name)
        })
        .unwrap_or(0)
}

fn has_windows_ui_name(font: FontRef<'_>) -> bool {
    font.name().ok().is_some_and(|names| {
        let strings = names.string_data();
        names.name_record().iter().any(|record| {
            record.string(strings).ok().is_some_and(|name| {
                matches!(name.to_string().as_str(), "Segoe UI" | "Yu Gothic UI")
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::windows_ui_face_index;

    #[test]
    fn invalid_data_falls_back_to_first_face() {
        assert_eq!(windows_ui_face_index(b"not a font"), 0);
    }

    #[test]
    fn single_font_falls_back_to_its_only_face() {
        assert_eq!(windows_ui_face_index(&sfnt("Other UI", 28)), 0);
    }

    #[test]
    fn collection_selects_the_first_matching_ui_face() {
        assert_eq!(
            windows_ui_face_index(&collection(&["Other UI", "Yu Gothic UI", "Segoe UI"])),
            1
        );
    }

    // These fixtures contain the minimal valid SFNT/name and TTC structures
    // required by read-fonts; no production font data is embedded in tests.
    fn sfnt(name: &str, name_table_offset: usize) -> Vec<u8> {
        let table = name_table(name);
        let mut font = Vec::with_capacity(28 + table.len());
        font.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        font.extend_from_slice(&1u16.to_be_bytes());
        font.extend_from_slice(&0u16.to_be_bytes());
        font.extend_from_slice(&0u16.to_be_bytes());
        font.extend_from_slice(&0u16.to_be_bytes());
        font.extend_from_slice(b"name");
        font.extend_from_slice(&0u32.to_be_bytes());
        font.extend_from_slice(&(name_table_offset as u32).to_be_bytes());
        font.extend_from_slice(&(table.len() as u32).to_be_bytes());
        font.extend_from_slice(&table);
        font
    }

    fn collection(names: &[&str]) -> Vec<u8> {
        let header_len = 12 + names.len() * 4;
        let table_lens: Vec<_> = names.iter().map(|name| name_table(name).len()).collect();
        let font_offsets: Vec<_> = table_lens
            .iter()
            .scan(header_len, |offset, table_len| {
                let font_offset = *offset;
                *offset += 28 + table_len;
                Some(font_offset)
            })
            .collect();
        let mut data = Vec::new();
        data.extend_from_slice(b"ttcf");
        data.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        data.extend_from_slice(&(names.len() as u32).to_be_bytes());
        for offset in &font_offsets {
            data.extend_from_slice(&(*offset as u32).to_be_bytes());
        }
        for (name, offset) in names.iter().zip(font_offsets) {
            data.extend_from_slice(&sfnt(name, offset + 28));
        }
        data
    }

    fn name_table(name: &str) -> Vec<u8> {
        let encoded: Vec<_> = name.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut table = Vec::new();
        table.extend_from_slice(&0u16.to_be_bytes());
        table.extend_from_slice(&1u16.to_be_bytes());
        table.extend_from_slice(&18u16.to_be_bytes());
        table.extend_from_slice(&3u16.to_be_bytes());
        table.extend_from_slice(&1u16.to_be_bytes());
        table.extend_from_slice(&0x0409u16.to_be_bytes());
        table.extend_from_slice(&1u16.to_be_bytes());
        table.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
        table.extend_from_slice(&0u16.to_be_bytes());
        table.extend_from_slice(&encoded);
        table
    }
}
