use thiserror::Error;

use crate::{alphabet::ALPHABET, decode::Js8DecodedFrame};

const ALPHANUMERIC: &[u8; 39] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ /@";
const N_BASE_GRID: u16 = 180 * 180;
const N_USER_GRID: u16 = N_BASE_GRID + 10;
const N_MAX_GRID: u16 = (1 << 15) - 1;

/// The frame kind encoded in the three JS8 information bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Js8FrameType {
    Heartbeat,
    Compound,
    CompoundDirected,
    Directed,
    Data,
    DataCompressed,
    Unknown(u8),
}

impl From<u8> for Js8FrameType {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Heartbeat,
            1 => Self::Compound,
            2 => Self::CompoundDirected,
            3 => Self::Directed,
            4 => Self::Data,
            6 => Self::DataCompressed,
            other => Self::Unknown(other),
        }
    }
}

/// A recognized JS8 directed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Js8Command {
    pub code: u8,
    pub name: String,
    pub number: Option<i8>,
}

/// Errors returned when constructing a semantic JS8 frame.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Js8MessageError {
    #[error("invalid JS8 callsign: {0}")]
    InvalidCallsign(String),
    #[error("invalid JS8 grid locator: {0}")]
    InvalidGrid(String),
    #[error("JS8 command number must be between -30 and 31")]
    InvalidCommandNumber,
    #[error("JS8 encoded payload must contain exactly 12 characters")]
    InvalidPayload,
}

/// Message-level interpretation of a decoded physical JS8 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Js8Message {
    Heartbeat {
        callsign: String,
        grid: Option<String>,
        cq: bool,
        subtype: u8,
    },
    Compound {
        callsign: String,
        grid: Option<String>,
        command: Option<Js8Command>,
        directed: bool,
    },
    Directed {
        from: String,
        to: String,
        command: Js8Command,
    },
    Data {
        encoded: String,
        compressed: bool,
    },
    Raw {
        frame_type: Js8FrameType,
        payload: String,
    },
}

/// Interpret the payload and frame type returned by the physical decoder.
///
/// The physical decoder always validates the CRC before this function is
/// called. Unknown and legacy compressed payloads are retained as `Raw`/`Data`
/// values so consumers can display or persist them without losing bytes.
pub fn decode_message(frame: &Js8DecodedFrame) -> Js8Message {
    match Js8FrameType::from(frame.frame_type) {
        Js8FrameType::Heartbeat => decode_compound_payload(frame, true, false),
        Js8FrameType::Compound => decode_compound_payload(frame, false, false),
        Js8FrameType::CompoundDirected => decode_compound_payload(frame, false, true),
        Js8FrameType::Directed => decode_directed_payload(frame),
        Js8FrameType::Data => Js8Message::Data {
            encoded: frame.message.clone(),
            compressed: false,
        },
        Js8FrameType::DataCompressed => Js8Message::Data {
            encoded: frame.message.clone(),
            compressed: true,
        },
        kind => Js8Message::Raw {
            frame_type: kind,
            payload: frame.message.clone(),
        },
    }
}

/// Encode a semantic message into the twelve-character JS8 payload and frame
/// type expected by [`crate::encode_tones`].
pub fn encode_message(message: &Js8Message) -> Result<(String, u8), Js8MessageError> {
    match message {
        Js8Message::Heartbeat {
            callsign,
            grid,
            cq,
            subtype,
        } => {
            let mut extra = grid
                .as_deref()
                .map(pack_grid)
                .transpose()?
                .unwrap_or(N_MAX_GRID);
            if *cq {
                extra |= 1 << 15;
            }
            Ok((pack_compound_payload(0, callsign, extra, *subtype)?, 0))
        }
        Js8Message::Compound {
            callsign,
            grid,
            command,
            directed,
        } => {
            let (extra, frame_type) = if let Some(command) = command {
                (N_USER_GRID + pack_command(command)?, 2)
            } else if let Some(grid) = grid {
                (pack_grid(grid)?, 1)
            } else {
                (N_MAX_GRID, 1)
            };
            let frame_type = if *directed { 2 } else { frame_type };
            Ok((
                pack_compound_payload(frame_type, callsign, extra, 0)?,
                frame_type,
            ))
        }
        Js8Message::Directed { from, to, command } => {
            let (from, portable_from) = pack_callsign(from)?;
            let (to, portable_to) = pack_callsign(to)?;
            let number = command
                .number
                .map(|value| {
                    if !(-30..=31).contains(&value) {
                        Err(Js8MessageError::InvalidCommandNumber)
                    } else {
                        Ok((value + 31) as u8)
                    }
                })
                .transpose()?
                .unwrap_or(31);
            let bits = ((3_u128 << 69)
                | (u128::from(from) << 41)
                | (u128::from(to) << 13)
                | (u128::from(command.code & 0x1f) << 8)
                | (u128::from(portable_from) << 7)
                | (u128::from(portable_to) << 6)
                | u128::from(number))
                & ((1_u128 << 72) - 1);
            Ok((pack_bits(bits), 3))
        }
        Js8Message::Data {
            encoded,
            compressed,
        } => {
            validate_payload(encoded)?;
            Ok((encoded.clone(), if *compressed { 6 } else { 4 }))
        }
        Js8Message::Raw {
            frame_type,
            payload,
        } => {
            validate_payload(payload)?;
            let frame_type = match frame_type {
                Js8FrameType::Unknown(value) => *value,
                Js8FrameType::Heartbeat => 0,
                Js8FrameType::Compound => 1,
                Js8FrameType::CompoundDirected => 2,
                Js8FrameType::Directed => 3,
                Js8FrameType::Data => 4,
                Js8FrameType::DataCompressed => 6,
            };
            Ok((payload.clone(), frame_type))
        }
    }
}

fn decode_compound_payload(frame: &Js8DecodedFrame, heartbeat: bool, directed: bool) -> Js8Message {
    let bits = payload_bits(&frame.message);
    let callsign = unpack_alphanumeric50(read_bits(&bits, 3, 50));
    let extra = ((read_bits(&bits, 53, 11) as u16) << 5) | read_bits(&bits, 64, 5) as u16;
    let subtype = read_bits(&bits, 69, 3) as u8;

    if heartbeat {
        return Js8Message::Heartbeat {
            callsign,
            grid: unpack_grid(extra & 0x7fff),
            cq: extra & 0x8000 != 0,
            subtype,
        };
    }

    Js8Message::Compound {
        callsign,
        grid: if extra <= N_BASE_GRID {
            unpack_grid(extra)
        } else {
            None
        },
        command: if (N_USER_GRID..N_MAX_GRID).contains(&extra) {
            Some(unpack_compound_command(extra - N_USER_GRID))
        } else {
            None
        },
        directed,
    }
}

fn decode_directed_payload(frame: &Js8DecodedFrame) -> Js8Message {
    let bits = payload_bits(&frame.message);
    let from = unpack_callsign(read_bits(&bits, 3, 28) as u32, read_bits(&bits, 64, 1) != 0);
    let to = unpack_callsign(
        read_bits(&bits, 31, 28) as u32,
        read_bits(&bits, 65, 1) != 0,
    );
    let code = read_bits(&bits, 59, 5) as u8;
    let number = read_bits(&bits, 66, 6) as i8 - 31;

    Js8Message::Directed {
        from,
        to,
        command: Js8Command {
            code,
            name: command_name(code).to_owned(),
            number: if code == 0 || code == 25 || code == 29 {
                Some(number)
            } else {
                None
            },
        },
    }
}

fn pack_compound_payload(
    frame_type: u8,
    callsign: &str,
    extra: u16,
    subtype: u8,
) -> Result<String, Js8MessageError> {
    let callsign = pack_alphanumeric50(callsign)?;
    let bits = (u128::from(frame_type & 7) << 69)
        | (u128::from(callsign) << 19)
        | (u128::from(extra >> 5) << 8)
        | (u128::from(extra & 0x1f) << 3)
        | u128::from(subtype & 7);
    Ok(pack_bits(bits))
}

fn pack_bits(value: u128) -> String {
    let mut output = String::with_capacity(12);
    for index in 0..12 {
        let shift = 66 - index * 6;
        let digit = ((value >> shift) & 0x3f) as usize;
        output.push(ALPHABET[digit] as char);
    }
    output
}

fn validate_payload(payload: &str) -> Result<(), Js8MessageError> {
    if payload.len() != 12 || !payload.bytes().all(|byte| ALPHABET.contains(&byte)) {
        return Err(Js8MessageError::InvalidPayload);
    }
    Ok(())
}

fn pack_alphanumeric50(value: &str) -> Result<u64, Js8MessageError> {
    let mut word = value.to_ascii_uppercase();
    if word.len() > 3 && word.as_bytes().get(3).is_none_or(|&byte| byte != b'/') {
        word.insert(3, ' ');
    }
    if word.len() > 7 && word.as_bytes().get(7).is_none_or(|&byte| byte != b'/') {
        word.insert(7, ' ');
    }
    if word.len() > 11 {
        return Err(Js8MessageError::InvalidCallsign(value.to_owned()));
    }
    word.extend(std::iter::repeat_n(' ', 11 - word.len()));
    if !word.bytes().all(|byte| ALPHANUMERIC.contains(&byte)) {
        return Err(Js8MessageError::InvalidCallsign(value.to_owned()));
    }

    let digits = word.as_bytes().iter().map(|&byte| {
        ALPHANUMERIC
            .iter()
            .position(|&candidate| candidate == byte)
            .unwrap() as u64
    });
    let digits: Vec<_> = digits.collect();
    let mut packed = digits[0];
    for &digit in &digits[1..3] {
        packed = packed * 38 + digit;
    }
    packed = packed * 2 + u64::from(digits[3] == 37);
    for &digit in &digits[4..7] {
        packed = packed * 38 + digit;
    }
    packed = packed * 2 + u64::from(digits[7] == 37);
    for &digit in &digits[8..11] {
        packed = packed * 38 + digit;
    }
    Ok(packed)
}

fn pack_callsign(value: &str) -> Result<(u32, bool), Js8MessageError> {
    let mut callsign = value.to_ascii_uppercase();
    let portable = callsign.ends_with("/P");
    if portable {
        callsign.truncate(callsign.len() - 2);
    }
    if !(2..=6).contains(&callsign.len())
        || !callsign
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'/')
    {
        return Err(Js8MessageError::InvalidCallsign(value.to_owned()));
    }
    let mut padded = callsign;
    padded.extend(std::iter::repeat_n(' ', 6 - padded.len()));
    let digits: Vec<_> = padded
        .bytes()
        .map(|byte| {
            ALPHANUMERIC
                .iter()
                .position(|&candidate| candidate == byte)
                .unwrap_or(0) as u32
        })
        .collect();
    let mut packed = digits[0];
    packed = packed * 36 + digits[1];
    packed = packed * 10 + digits[2];
    for &digit in &digits[3..6] {
        packed = packed * 27 + digit - 10;
    }
    Ok((packed, portable))
}

fn pack_grid(value: &str) -> Result<u16, Js8MessageError> {
    let bytes = value.as_bytes();
    if bytes.len() < 4
        || !(b'A'..=b'R').contains(&bytes[0])
        || !(b'A'..=b'R').contains(&bytes[1])
        || !bytes[2].is_ascii_digit()
        || !bytes[3].is_ascii_digit()
    {
        return Err(Js8MessageError::InvalidGrid(value.to_owned()));
    }
    let longitude = 179 - (u16::from(bytes[0] - b'A') * 10 + u16::from(bytes[2] - b'0'));
    let latitude = u16::from(bytes[1] - b'A') * 10 + u16::from(bytes[3] - b'0');
    Ok(longitude * 180 + latitude)
}

fn pack_command(command: &Js8Command) -> Result<u16, Js8MessageError> {
    if command.code == 25 || command.code == 29 {
        let number = command
            .number
            .ok_or(Js8MessageError::InvalidCommandNumber)?;
        if !(-30..=31).contains(&number) {
            return Err(Js8MessageError::InvalidCommandNumber);
        }
        return Ok(0x80 | u16::from((number + 31) as u8));
    }
    Ok(u16::from(command.code & 0x7f))
}

fn payload_bits(payload: &str) -> [u8; 72] {
    let mut bits = [0_u8; 72];
    for (index, byte) in payload.bytes().enumerate().take(12) {
        let value = ALPHABET
            .iter()
            .position(|&candidate| candidate == byte)
            .unwrap_or(0) as u8;
        for shift in 0..6 {
            bits[index * 6 + shift] = (value >> (5 - shift)) & 1;
        }
    }
    bits
}

fn read_bits(bits: &[u8; 72], start: usize, length: usize) -> u64 {
    bits[start..start + length]
        .iter()
        .fold(0_u64, |value, &bit| (value << 1) | u64::from(bit))
}

fn unpack_alphanumeric50(mut value: u64) -> String {
    let mut word = [b' '; 11];
    word[10] = ALPHANUMERIC[(value % 38) as usize];
    value /= 38;
    word[9] = ALPHANUMERIC[(value % 38) as usize];
    value /= 38;
    word[8] = ALPHANUMERIC[(value % 38) as usize];
    value /= 38;
    word[7] = if value % 2 == 1 { b'/' } else { b' ' };
    value /= 2;
    word[6] = ALPHANUMERIC[(value % 38) as usize];
    value /= 38;
    word[5] = ALPHANUMERIC[(value % 38) as usize];
    value /= 38;
    word[4] = ALPHANUMERIC[(value % 38) as usize];
    value /= 38;
    word[3] = if value % 2 == 1 { b'/' } else { b' ' };
    value /= 2;
    word[2] = ALPHANUMERIC[(value % 38) as usize];
    value /= 38;
    word[1] = ALPHANUMERIC[(value % 38) as usize];
    value /= 38;
    word[0] = ALPHANUMERIC[(value % 39) as usize];
    String::from_utf8_lossy(&word).replace(' ', "")
}

fn unpack_callsign(mut value: u32, portable: bool) -> String {
    let mut word = [b' '; 6];
    word[5] = ALPHANUMERIC[(value % 27 + 10) as usize];
    value /= 27;
    word[4] = ALPHANUMERIC[(value % 27 + 10) as usize];
    value /= 27;
    word[3] = ALPHANUMERIC[(value % 27 + 10) as usize];
    value /= 27;
    word[2] = ALPHANUMERIC[(value % 10) as usize];
    value /= 10;
    word[1] = ALPHANUMERIC[(value % 36) as usize];
    value /= 36;
    word[0] = ALPHANUMERIC[value as usize];

    let mut callsign = String::from_utf8_lossy(&word).trim().to_owned();
    if callsign.starts_with("3D0") {
        callsign = format!("3DA0{}", &callsign[3..]);
    } else if callsign.len() > 1
        && callsign.starts_with('Q')
        && callsign.as_bytes()[1].is_ascii_uppercase()
    {
        callsign = format!("3X{}", &callsign[1..]);
    }
    if portable {
        callsign.push_str("/P");
    }
    callsign
}

fn unpack_grid(value: u16) -> Option<String> {
    if value > N_BASE_GRID {
        return None;
    }
    let latitude = value % 180;
    let longitude = 179 - value / 180;
    let field_lon = longitude / 10;
    let field_lat = latitude / 10;
    let square_lon = longitude % 10;
    let square_lat = latitude % 10;
    Some(format!(
        "{}{}{}{}",
        (b'A' + field_lon as u8) as char,
        (b'A' + field_lat as u8) as char,
        (b'0' + square_lon as u8) as char,
        (b'0' + square_lat as u8) as char
    ))
}

fn unpack_compound_command(value: u16) -> Js8Command {
    let value = value as u8;
    let is_snr = value & 0x80 != 0;
    let code = if is_snr { 25 } else { value & 0x7f };
    let number = (value & 0x3f) as i8 - 31;
    Js8Command {
        code,
        name: command_name(code).to_owned(),
        number: if is_snr || code == 0 || code == 25 || code == 29 {
            Some(number)
        } else {
            None
        },
    }
}

fn command_name(code: u8) -> &'static str {
    match code {
        0 => "SNR?",
        1 => "DIT DIT",
        2 => "NACK",
        3 => "HEARING?",
        4 => "GRID?",
        5 => ">",
        6 => "STATUS?",
        7 => "STATUS",
        8 => "HEARING",
        9 => "MSG",
        10 => "MSG TO:",
        11 => "QUERY",
        12 => "QUERY MSGS",
        13 => "QUERY CALL",
        14 => "ACK",
        15 => "GRID",
        16 => "INFO?",
        17 => "INFO",
        18 => "FB",
        19 => "HW CPY?",
        20 => "SK",
        21 => "RR",
        22 => "QSL?",
        23 => "QSL",
        24 => "CMD",
        25 => "SNR",
        26 => "NO",
        27 => "YES",
        28 => "73",
        29 => "HEARTBEAT SNR",
        30 => "AGN?",
        31 => "TEXT",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_message, encode_message, Js8Command, Js8FrameType, Js8Message};
    use crate::Js8DecodedFrame;

    #[test]
    fn classifies_frame_types_and_preserves_data_payloads() {
        let frame = Js8DecodedFrame {
            message: "0123456789AB".to_owned(),
            frame_type: 6,
        };
        assert_eq!(
            decode_message(&frame),
            Js8Message::Data {
                encoded: frame.message,
                compressed: true,
            }
        );
        assert_eq!(Js8FrameType::from(5), Js8FrameType::Unknown(5));
    }

    #[test]
    fn decodes_oracle_directed_frame_layout() {
        let frame = Js8DecodedFrame {
            message: "000000000000".to_owned(),
            frame_type: 3,
        };
        assert_eq!(
            decode_message(&frame),
            Js8Message::Directed {
                from: "000AAA".to_owned(),
                to: "000AAA".to_owned(),
                command: Js8Command {
                    code: 0,
                    name: "SNR?".to_owned(),
                    number: Some(-31),
                },
            }
        );
    }

    #[test]
    fn semantic_frames_round_trip_through_the_72_bit_payload() {
        let messages = [
            Js8Message::Heartbeat {
                callsign: "KN4CRD".to_owned(),
                grid: Some("EM73".to_owned()),
                cq: false,
                subtype: 0,
            },
            Js8Message::Compound {
                callsign: "KN4CRD".to_owned(),
                grid: Some("EM73".to_owned()),
                command: None,
                directed: false,
            },
            Js8Message::Directed {
                from: "KN4CRD".to_owned(),
                to: "AB1CD".to_owned(),
                command: Js8Command {
                    code: 23,
                    name: "QSL".to_owned(),
                    number: None,
                },
            },
        ];

        for message in messages {
            let (payload, frame_type) = encode_message(&message).unwrap();
            let decoded = decode_message(&Js8DecodedFrame {
                message: payload,
                frame_type,
            });
            assert_eq!(decoded, message);
        }
    }
}
