use crate::term::cell::Cell;
use crate::vte::ansi::Color;

use super::rowcolumn_diacritics::diacritic_index;

pub const PLACEHOLDER: char = '\u{10eeee}';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceholderCell {
    pub image_id: u32,
    pub placement_id: u32,
    pub row: u8,
    pub column: u8,
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
    let row = marks.first().copied().and_then(diacritic_index);
    let column = marks.get(1).copied().and_then(diacritic_index);
    let high = marks.get(2).copied().and_then(diacritic_index);
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
    fn inherits_omitted_columns_left_to_right() {
        let first = cell(Color::Indexed(42), &['\u{30d}']);
        let first = decode_placeholder(&first, None).unwrap();
        let second = cell(Color::Indexed(42), &[]);
        let second = decode_placeholder(&second, Some(first)).unwrap();
        assert_eq!((second.row, second.column), (1, 1));
    }

    #[test]
    fn rejects_missing_identity_or_row() {
        assert!(decode_placeholder(&Cell { c: PLACEHOLDER, ..Default::default() }, None).is_none());
        assert!(decode_placeholder(&cell(Color::Indexed(1), &[]), None).is_none());
    }
}
