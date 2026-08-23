use crate::term::cell::Cell;
use crate::vte::ansi::Color;

use super::rowcolumn_diacritics::diacritic_index;

pub const PLACEHOLDER: char = '\u{10eeee}';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceholderCell {
    pub image_id: u32,
    pub placement_id: u32,
    pub row: u16,
    pub column: u16,
    foreground: Color,
    underline: Option<Color>,
}

pub fn decode_placeholder(
    cell: &Cell,
    previous: Option<PlaceholderCell>,
) -> Option<PlaceholderCell> {
    if cell.c != PLACEHOLDER {
        return None;
    }
    let foreground = cell.fg;
    let underline = cell.underline_color();
    let low_image_id = color_id(foreground)?;
    let placement_id = underline.and_then(color_id).unwrap_or(0);
    let marks = cell.zerowidth().unwrap_or_default();
    let row = match marks.first() {
        Some(mark) => Some(diacritic_index(*mark)?),
        None => None,
    };
    let column = match marks.get(1) {
        Some(mark) => Some(diacritic_index(*mark)?),
        None => None,
    };
    let high = match marks.get(2) {
        Some(mark) => Some(u8::try_from(diacritic_index(*mark)?).ok()?),
        None => None,
    };
    let same_identity = previous
        .filter(|previous| previous.foreground == foreground && previous.underline == underline);

    let explicit_row = row.is_some();
    let row = row.or_else(|| same_identity.map(|previous| previous.row))?;
    let column = match column {
        Some(column) => column,
        None => match same_identity
            .filter(|previous| previous.row == row)
            .and_then(|previous| previous.column.checked_add(1))
        {
            Some(column) => column,
            None if explicit_row => 0,
            None => return None,
        },
    };
    let high = match high {
        Some(high) => high,
        None => same_identity
            .filter(|previous| {
                previous.row == row && previous.column.checked_add(1) == Some(column)
            })
            .map_or(0, |previous| (previous.image_id >> 24) as u8),
    };

    Some(PlaceholderCell {
        image_id: low_image_id | (u32::from(high) << 24),
        placement_id,
        row,
        column,
        foreground,
        underline,
    })
}

fn color_id(color: Color) -> Option<u32> {
    match color {
        Color::Indexed(index) => Some(u32::from(index)),
        Color::Spec(rgb) => {
            Some((u32::from(rgb.r) << 16) | (u32::from(rgb.g) << 8) | u32::from(rgb.b))
        },
        Color::Named(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::rowcolumn_diacritics::ROW_COLUMN_DIACRITICS;
    use crate::vte::ansi::Rgb;

    fn cell(image: Color, marks: &[char]) -> Cell {
        let mut cell = Cell { c: PLACEHOLDER, fg: image, ..Default::default() };
        for mark in marks {
            cell.push_zerowidth(*mark);
        }
        cell
    }

    #[test]
    fn decodes_full_image_and_tile_identity() {
        let cell = cell(Color::Spec(Rgb { r: 0, g: 0, b: 42 }), &['\u{305}', '\u{30d}', '\u{30e}']);
        let placeholder = decode_placeholder(&cell, None).unwrap();
        assert_eq!(placeholder.image_id, 33_554_474);
        assert_eq!((placeholder.row, placeholder.column), (0, 1));
    }

    #[test]
    fn decodes_indexed_and_rgb_placement_ids_from_underline_color() {
        let mut indexed = cell(Color::Indexed(42), &[ROW_COLUMN_DIACRITICS[0]]);
        indexed.set_underline_color(Some(Color::Indexed(7)));
        assert_eq!(decode_placeholder(&indexed, None).unwrap().placement_id, 7);

        let mut rgb = cell(Color::Indexed(42), &[ROW_COLUMN_DIACRITICS[0]]);
        rgb.set_underline_color(Some(Color::Spec(Rgb { r: 1, g: 2, b: 3 })));
        assert_eq!(decode_placeholder(&rgb, None).unwrap().placement_id, 0x010203);
    }

    #[test]
    fn inherits_omitted_columns_left_to_right() {
        let first = cell(Color::Indexed(42), &['\u{30d}']);
        let first = decode_placeholder(&first, None).unwrap();
        let second = cell(Color::Indexed(42), &[]);
        let second = decode_placeholder(&second, Some(first)).unwrap();
        assert_eq!((second.row, second.column), (1, 1));
    }

    #[test]
    fn applies_all_protocol_inheritance_conditions() {
        let identity = Color::Indexed(42);
        let first = cell(identity, &[
            ROW_COLUMN_DIACRITICS[0],
            ROW_COLUMN_DIACRITICS[0],
            ROW_COLUMN_DIACRITICS[2],
        ]);
        let first = decode_placeholder(&first, None).unwrap();

        let inherited = decode_placeholder(&cell(identity, &[]), Some(first)).unwrap();
        assert_eq!((inherited.row, inherited.column, inherited.image_id >> 24), (0, 1, 2));
        let row_only =
            decode_placeholder(&cell(identity, &[ROW_COLUMN_DIACRITICS[0]]), Some(first)).unwrap();
        assert_eq!((row_only.column, row_only.image_id >> 24), (1, 2));
        let adjacent = decode_placeholder(
            &cell(identity, &[ROW_COLUMN_DIACRITICS[0], ROW_COLUMN_DIACRITICS[1]]),
            Some(first),
        )
        .unwrap();
        assert_eq!(adjacent.image_id >> 24, 2);
        let nonadjacent = decode_placeholder(
            &cell(identity, &[ROW_COLUMN_DIACRITICS[0], ROW_COLUMN_DIACRITICS[2]]),
            Some(first),
        )
        .unwrap();
        assert_eq!(nonadjacent.image_id >> 24, 0);
        let different_row =
            decode_placeholder(&cell(identity, &[ROW_COLUMN_DIACRITICS[1]]), Some(first)).unwrap();
        assert_eq!((different_row.column, different_row.image_id >> 24), (0, 0));
        let different_identity =
            decode_placeholder(&cell(Color::Indexed(43), &[ROW_COLUMN_DIACRITICS[0]]), Some(first))
                .unwrap();
        assert_eq!((different_identity.column, different_identity.image_id >> 24), (0, 0));
        let mut different_underline = cell(identity, &[]);
        different_underline.set_underline_color(Some(Color::Indexed(1)));
        assert!(decode_placeholder(&different_underline, Some(first)).is_none());
    }

    #[test]
    fn supports_entire_row_and_column_table_but_bounds_high_byte() {
        let final_mark = ROW_COLUMN_DIACRITICS[296];
        let endpoint = cell(Color::Indexed(1), &[final_mark, final_mark]);
        let placeholder = decode_placeholder(&endpoint, None).unwrap();
        assert_eq!((placeholder.row, placeholder.column), (296, 296));

        let oversized_high = cell(Color::Indexed(1), &[
            ROW_COLUMN_DIACRITICS[0],
            ROW_COLUMN_DIACRITICS[0],
            ROW_COLUMN_DIACRITICS[256],
        ]);
        assert!(decode_placeholder(&oversized_high, None).is_none());
        assert!(decode_placeholder(&cell(Color::Indexed(1), &['\u{301}']), None).is_none());
    }

    #[test]
    fn rejects_missing_identity_or_row() {
        assert!(decode_placeholder(&Cell { c: PLACEHOLDER, ..Default::default() }, None).is_none());
        assert!(decode_placeholder(&cell(Color::Indexed(1), &[]), None).is_none());
    }
}
