//! SVG path data (`d`) into a lyon path: the parser the console draws the
//! symbol library with, here (no UI) so the library's tests read every
//! symbol file's paths with it.

use lyon::math::{Angle, Vector, point as lpoint, vector};
use lyon::path::traits::SvgPathBuilder;
use lyon::path::{ArcFlags, Path};

/// SVG path data into a lyon path. Stops at the first thing it cannot
/// read, keeping what came before.
pub fn parse_svg(data: &str) -> Path {
    parse(data).0
}

/// SVG path data read to its end, or the byte offset where reading stopped.
pub fn parse_svg_strict(data: &str) -> Result<Path, usize> {
    let (path, at) = parse(data);
    if data[at..].trim().is_empty() {
        Ok(path)
    } else {
        Err(at)
    }
}

fn parse(data: &str) -> (Path, usize) {
    let mut scan = Scan {
        bytes: data.as_bytes(),
        at: 0,
    };
    let mut builder = Path::svg_builder();
    let mut command = b'M';
    loop {
        if let Some(next) = scan.command() {
            command = next;
            if command.eq_ignore_ascii_case(&b'z') {
                builder.close();
                continue;
            }
        } else if !scan.at_number() || command.eq_ignore_ascii_case(&b'z') {
            break;
        }
        let read = match command {
            b'M' => scan.point().map(|p| {
                builder.move_to(p);
                command = b'L';
            }),
            b'm' => scan.vector().map(|v| {
                builder.relative_move_to(v);
                command = b'l';
            }),
            b'L' => scan.point().map(|p| {
                builder.line_to(p);
            }),
            b'l' => scan.vector().map(|v| {
                builder.relative_line_to(v);
            }),
            b'H' => scan.number().map(|x| builder.horizontal_line_to(x)),
            b'h' => scan
                .number()
                .map(|x| builder.relative_horizontal_line_to(x)),
            b'V' => scan.number().map(|y| builder.vertical_line_to(y)),
            b'v' => scan.number().map(|y| builder.relative_vertical_line_to(y)),
            b'C' => scan.points3().map(|(a, b, to)| {
                builder.cubic_bezier_to(a, b, to);
            }),
            b'c' => scan.vectors3().map(|(a, b, to)| {
                builder.relative_cubic_bezier_to(a, b, to);
            }),
            b'S' => scan.points2().map(|(b, to)| {
                builder.smooth_cubic_bezier_to(b, to);
            }),
            b's' => scan.vectors2().map(|(b, to)| {
                builder.smooth_relative_cubic_bezier_to(b, to);
            }),
            b'Q' => scan.points2().map(|(c, to)| {
                builder.quadratic_bezier_to(c, to);
            }),
            b'q' => scan.vectors2().map(|(c, to)| {
                builder.relative_quadratic_bezier_to(c, to);
            }),
            b'T' => scan.point().map(|to| {
                builder.smooth_quadratic_bezier_to(to);
            }),
            b't' => scan.vector().map(|to| {
                builder.smooth_relative_quadratic_bezier_to(to);
            }),
            b'A' | b'a' => scan.arc().map(|(radii, rotation, flags, x, y)| {
                if command == b'A' {
                    builder.arc_to(radii, rotation, flags, lpoint(x, y));
                } else {
                    builder.relative_arc_to(radii, rotation, flags, vector(x, y));
                }
            }),
            _ => None,
        };
        if read.is_none() {
            break;
        }
    }
    (builder.build(), scan.at)
}

struct Scan<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Scan<'_> {
    fn skip(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            self.at += 1;
        }
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let byte = *self.bytes.get(self.at)?;
        (byte.is_ascii_alphabetic() && byte != b'e' && byte != b'E').then(|| {
            self.at += 1;
            byte
        })
    }

    fn at_number(&mut self) -> bool {
        self.skip();
        self.bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.'))
    }

    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.at;
        let mut end = start;
        let bytes = self.bytes;
        if matches!(bytes.get(end), Some(b'-' | b'+')) {
            end += 1;
        }
        let mut dot = false;
        while let Some(&b) = bytes.get(end) {
            if b.is_ascii_digit() {
                end += 1;
            } else if b == b'.' && !dot {
                dot = true;
                end += 1;
            } else {
                break;
            }
        }
        if matches!(bytes.get(end), Some(b'e' | b'E')) {
            let mut exponent = end + 1;
            if matches!(bytes.get(exponent), Some(b'-' | b'+')) {
                exponent += 1;
            }
            if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
                end = exponent;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
            }
        }
        let value = std::str::from_utf8(&bytes[start..end]).ok()?.parse().ok()?;
        self.at = end;
        Some(value)
    }

    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let byte = *self.bytes.get(self.at)?;
        let flag = match byte {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.at += 1;
        Some(flag)
    }

    fn point(&mut self) -> Option<lyon::math::Point> {
        Some(lpoint(self.number()?, self.number()?))
    }

    fn vector(&mut self) -> Option<Vector> {
        Some(vector(self.number()?, self.number()?))
    }

    fn points2(&mut self) -> Option<(lyon::math::Point, lyon::math::Point)> {
        Some((self.point()?, self.point()?))
    }

    fn points3(&mut self) -> Option<(lyon::math::Point, lyon::math::Point, lyon::math::Point)> {
        Some((self.point()?, self.point()?, self.point()?))
    }

    fn vectors2(&mut self) -> Option<(Vector, Vector)> {
        Some((self.vector()?, self.vector()?))
    }

    fn vectors3(&mut self) -> Option<(Vector, Vector, Vector)> {
        Some((self.vector()?, self.vector()?, self.vector()?))
    }

    fn arc(&mut self) -> Option<(Vector, Angle, ArcFlags, f32, f32)> {
        let radii = self.vector()?;
        let rotation = Angle::degrees(self.number()?);
        let large_arc = self.flag()?;
        let sweep = self.flag()?;
        let x = self.number()?;
        let y = self.number()?;
        Some((radii, rotation, ArcFlags { large_arc, sweep }, x, y))
    }
}
