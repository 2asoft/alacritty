use std::{mem, str};

use crate::index::Point;

pub const MAX_GRAPHICS_CONTROL_BYTES: usize = 4096;
pub const MAX_GRAPHICS_PAYLOAD_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    Animate,
    ComposeFrame,
    Delete,
    TransmitFrame,
    Place,
    Query,
    #[default]
    Transmit,
    TransmitAndPlace,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    Rgb,
    #[default]
    Rgba,
    Png,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transmission {
    #[default]
    Direct,
    File,
    TemporaryFile,
    SharedMemory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Zlib,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteTarget(pub u8);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    pub action: Option<Action>,
    pub quiet: Option<u8>,
    pub format: Option<Format>,
    pub transmission: Option<Transmission>,
    pub compression: Option<Compression>,
    pub delete: Option<DeleteTarget>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub data_size: Option<u32>,
    pub data_offset: Option<u32>,
    pub image_id: Option<u32>,
    pub image_number: Option<u32>,
    pub placement_id: Option<u32>,
    pub more: Option<bool>,
    pub usage: Option<u32>,
    pub x: Option<u32>,
    pub y: Option<u32>,
    pub crop_width: Option<u32>,
    pub crop_height: Option<u32>,
    pub x_offset: Option<u32>,
    pub y_offset: Option<u32>,
    pub columns: Option<u32>,
    pub rows: Option<u32>,
    pub cursor_policy: Option<u32>,
    pub unicode_placeholder: Option<u32>,
    pub z_index: Option<i32>,
    pub parent_image_id: Option<u32>,
    pub parent_placement_id: Option<u32>,
    pub horizontal_offset: Option<i32>,
    pub vertical_offset: Option<i32>,
    pub anchor: Option<Point>,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsError {
    InvalidControl,
    ControlTooLarge,
    PayloadTooLarge,
    Invalid,
    TooLarge,
    NoSpace,
    Decode,
    Unsupported,
    LocalTransmissionDisabled,
    NotFound,
    Io,
}

impl GraphicsError {
    pub fn protocol_code(self) -> &'static str {
        match self {
            Self::InvalidControl | Self::Invalid => "EINVAL",
            Self::ControlTooLarge | Self::PayloadTooLarge | Self::TooLarge => "E2BIG",
            Self::NoSpace => "ENOSPC",
            Self::Decode => "EBADPNG",
            Self::Unsupported => "ENOTSUP",
            Self::LocalTransmissionDisabled => "EACCES",
            Self::NotFound => "ENOENT",
            Self::Io => "EIO",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Prefix,
    Control,
    Payload,
    Overflow(GraphicsError),
    Ignore,
}

#[derive(Debug, Default)]
pub struct GraphicsApcParser {
    state: State,
    control: Vec<u8>,
    payload: Vec<u8>,
}

impl GraphicsApcParser {
    pub fn start(&mut self) {
        self.state = State::Prefix;
        self.control.clear();
        self.payload.clear();
    }

    pub fn put(&mut self, byte: u8) {
        match self.state {
            State::Prefix => {
                self.state = if byte == b'G' { State::Control } else { State::Ignore };
            },
            State::Control if byte == b';' => self.state = State::Payload,
            State::Control => {
                if self.control.len() == MAX_GRAPHICS_CONTROL_BYTES {
                    self.state = State::Overflow(GraphicsError::ControlTooLarge);
                } else {
                    self.control.push(byte);
                }
            },
            State::Payload => {
                if self.payload.len() == MAX_GRAPHICS_PAYLOAD_BYTES {
                    self.state = State::Overflow(GraphicsError::PayloadTooLarge);
                } else {
                    self.payload.push(byte);
                }
            },
            State::Overflow(_) | State::Ignore => (),
        }
    }

    pub fn end(&mut self) -> Option<Result<Command, GraphicsError>> {
        let state = mem::take(&mut self.state);
        let result = match state {
            State::Ignore | State::Prefix => None,
            State::Overflow(error) => Some(Err(error)),
            State::Control | State::Payload => Some(self.parse_command()),
        };
        self.control.clear();
        self.payload.clear();
        result
    }

    fn parse_command(&self) -> Result<Command, GraphicsError> {
        let mut command = Command { payload: self.payload.clone(), ..Default::default() };
        if self.control.is_empty() {
            return Ok(command);
        }

        for property in self.control.split(|byte| *byte == b',') {
            let separator = property
                .iter()
                .position(|byte| *byte == b'=')
                .ok_or(GraphicsError::InvalidControl)?;
            let (key, value) = property.split_at(separator);
            let value = &value[1..];
            if key.len() != 1
                || !key[0].is_ascii_alphabetic()
                || value.is_empty()
                || value.contains(&b'=')
            {
                return Err(GraphicsError::InvalidControl);
            }

            let unsigned = || parse_u32(value);
            let signed = || parse_i32(value);
            match key[0] {
                b'a' => command.action = Some(parse_action(value)?),
                b'q' => command.quiet = Some(parse_range(value, 0..=2)? as u8),
                b'f' => command.format = Some(parse_format(value)?),
                b't' => command.transmission = Some(parse_transmission(value)?),
                b'o' if value == b"z" => command.compression = Some(Compression::Zlib),
                b'o' => return Err(GraphicsError::InvalidControl),
                b'd' => command.delete = Some(parse_delete(value)?),
                b's' => command.width = Some(unsigned()?),
                b'v' => command.height = Some(unsigned()?),
                b'S' => command.data_size = Some(unsigned()?),
                b'O' => command.data_offset = Some(unsigned()?),
                b'i' => command.image_id = Some(unsigned()?),
                b'I' => command.image_number = Some(unsigned()?),
                b'p' => command.placement_id = Some(unsigned()?),
                b'm' => command.more = Some(parse_range(value, 0..=1)? != 0),
                b'N' => command.usage = Some(unsigned()?),
                b'x' => command.x = Some(unsigned()?),
                b'y' => command.y = Some(unsigned()?),
                b'w' => command.crop_width = Some(unsigned()?),
                b'h' => command.crop_height = Some(unsigned()?),
                b'X' => command.x_offset = Some(unsigned()?),
                b'Y' => command.y_offset = Some(unsigned()?),
                b'c' => command.columns = Some(unsigned()?),
                b'r' => command.rows = Some(unsigned()?),
                b'C' => command.cursor_policy = Some(unsigned()?),
                b'U' => command.unicode_placeholder = Some(unsigned()?),
                b'z' => command.z_index = Some(signed()?),
                b'P' => command.parent_image_id = Some(unsigned()?),
                b'Q' => command.parent_placement_id = Some(unsigned()?),
                b'H' => command.horizontal_offset = Some(signed()?),
                b'V' => command.vertical_offset = Some(signed()?),
                _ => (),
            }
        }

        Ok(command)
    }

    pub fn abort(&mut self) {
        self.start();
    }
}

fn parse_u32(value: &[u8]) -> Result<u32, GraphicsError> {
    let value = str::from_utf8(value).map_err(|_| GraphicsError::InvalidControl)?;
    value.parse().map_err(|_| GraphicsError::InvalidControl)
}

fn parse_i32(value: &[u8]) -> Result<i32, GraphicsError> {
    let value = str::from_utf8(value).map_err(|_| GraphicsError::InvalidControl)?;
    value.parse().map_err(|_| GraphicsError::InvalidControl)
}

fn parse_range(value: &[u8], range: std::ops::RangeInclusive<u32>) -> Result<u32, GraphicsError> {
    let value = parse_u32(value)?;
    if range.contains(&value) { Ok(value) } else { Err(GraphicsError::InvalidControl) }
}

fn parse_action(value: &[u8]) -> Result<Action, GraphicsError> {
    match value {
        b"a" => Ok(Action::Animate),
        b"c" => Ok(Action::ComposeFrame),
        b"d" => Ok(Action::Delete),
        b"f" => Ok(Action::TransmitFrame),
        b"p" => Ok(Action::Place),
        b"q" => Ok(Action::Query),
        b"t" => Ok(Action::Transmit),
        b"T" => Ok(Action::TransmitAndPlace),
        _ => Err(GraphicsError::InvalidControl),
    }
}

fn parse_format(value: &[u8]) -> Result<Format, GraphicsError> {
    match value {
        b"24" => Ok(Format::Rgb),
        b"32" => Ok(Format::Rgba),
        b"100" => Ok(Format::Png),
        _ => Err(GraphicsError::InvalidControl),
    }
}

fn parse_transmission(value: &[u8]) -> Result<Transmission, GraphicsError> {
    match value {
        b"d" => Ok(Transmission::Direct),
        b"f" => Ok(Transmission::File),
        b"t" => Ok(Transmission::TemporaryFile),
        b"s" => Ok(Transmission::SharedMemory),
        _ => Err(GraphicsError::InvalidControl),
    }
}

fn parse_delete(value: &[u8]) -> Result<DeleteTarget, GraphicsError> {
    match value {
        [target] if b"aAcCfFiInNpPqQrRxXyYzZ".contains(target) => Ok(DeleteTarget(*target)),
        _ => Err(GraphicsError::InvalidControl),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> Option<Result<Command, GraphicsError>> {
        let mut parser = GraphicsApcParser::default();
        parser.start();
        input.iter().for_each(|byte| parser.put(*byte));
        parser.end()
    }

    #[test]
    fn parses_all_control_types_and_payload() {
        let command = parse(b"Ga=T,q=2,f=100,t=s,o=z,d=Z,s=10,v=20,S=30,O=40,i=50,I=60,p=70,m=1,N=1,x=2,y=3,w=4,h=5,X=6,Y=7,c=8,r=9,C=1,U=1,z=-10,P=11,Q=12,H=-13,V=14;AAAA")
            .unwrap()
            .unwrap();

        assert_eq!(command.action, Some(Action::TransmitAndPlace));
        assert_eq!(command.quiet, Some(2));
        assert_eq!(command.format, Some(Format::Png));
        assert_eq!(command.transmission, Some(Transmission::SharedMemory));
        assert_eq!(command.compression, Some(Compression::Zlib));
        assert_eq!(command.delete, Some(DeleteTarget(b'Z')));
        assert_eq!(command.width, Some(10));
        assert_eq!(command.height, Some(20));
        assert_eq!(command.data_size, Some(30));
        assert_eq!(command.data_offset, Some(40));
        assert_eq!(command.image_id, Some(50));
        assert_eq!(command.image_number, Some(60));
        assert_eq!(command.placement_id, Some(70));
        assert_eq!(command.more, Some(true));
        assert_eq!(command.usage, Some(1));
        assert_eq!(command.x, Some(2));
        assert_eq!(command.y, Some(3));
        assert_eq!(command.crop_width, Some(4));
        assert_eq!(command.crop_height, Some(5));
        assert_eq!(command.x_offset, Some(6));
        assert_eq!(command.y_offset, Some(7));
        assert_eq!(command.columns, Some(8));
        assert_eq!(command.rows, Some(9));
        assert_eq!(command.cursor_policy, Some(1));
        assert_eq!(command.unicode_placeholder, Some(1));
        assert_eq!(command.z_index, Some(-10));
        assert_eq!(command.parent_image_id, Some(11));
        assert_eq!(command.parent_placement_id, Some(12));
        assert_eq!(command.horizontal_offset, Some(-13));
        assert_eq!(command.vertical_offset, Some(14));
        assert_eq!(command.payload, b"AAAA");
    }

    #[test]
    fn final_duplicate_key_wins_and_unknown_keys_are_ignored() {
        let command = parse(b"Gi=1,k=future,i=2;").unwrap().unwrap();
        assert_eq!(command.image_id, Some(2));
    }

    #[test]
    fn ignores_non_kitty_apc() {
        assert_eq!(parse(b"not-kitty"), None);
    }

    #[test]
    fn rejects_malformed_and_out_of_range_values() {
        for input in [
            b"Gi".as_slice(),
            b"Gi=".as_slice(),
            b"Gii=1".as_slice(),
            b"Gi=-1".as_slice(),
            b"Gi=4294967296".as_slice(),
            b"Gz=2147483648".as_slice(),
            b"Ga=x".as_slice(),
            b"Gq=3".as_slice(),
            b"Gm=2".as_slice(),
        ] {
            assert_eq!(parse(input), Some(Err(GraphicsError::InvalidControl)), "{input:?}");
        }
    }

    #[test]
    fn bounds_control_and_payload_independently() {
        let mut control = vec![b'G'];
        control.extend([b'i'; MAX_GRAPHICS_CONTROL_BYTES + 1]);
        assert_eq!(parse(&control), Some(Err(GraphicsError::ControlTooLarge)));

        let mut payload = b"G;".to_vec();
        payload.extend([b'A'; MAX_GRAPHICS_PAYLOAD_BYTES + 1]);
        assert_eq!(parse(&payload), Some(Err(GraphicsError::PayloadTooLarge)));
    }

    #[test]
    fn cancellation_discards_partial_command_and_reuses_parser() {
        let mut parser = GraphicsApcParser::default();
        parser.start();
        b"Gi=1;AAAA".iter().for_each(|byte| parser.put(*byte));
        parser.abort();
        assert_eq!(parser.end(), None);

        parser.start();
        b"Gi=2;".iter().for_each(|byte| parser.put(*byte));
        assert_eq!(parser.end().unwrap().unwrap().image_id, Some(2));
    }
}
