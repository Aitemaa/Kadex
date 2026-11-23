//! Lucide-style stroked icons on a 24-unit grid (the Kavox family icon language).
//!
//! Every icon is drawn with a 2.0-unit stroke, round caps and round joins. Arcs are pre-converted
//! to cubic Béziers so both frontends (Cairo on Wayland, GDI+ on Windows) share one tiny data
//! table and need no SVG parser.

pub const GRID: f32 = 24.0;
pub const STROKE: f32 = 2.0;

#[derive(Debug, Clone, Copy)]
pub enum Seg {
    M(f32, f32),
    L(f32, f32),
    /// Cubic Bézier: control 1, control 2, end point.
    C(f32, f32, f32, f32, f32, f32),
    Z,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Plus,
    Close,
    Archive,
    Pin,
}

impl Icon {
    pub const fn segments(self) -> &'static [Seg] {
        match self {
            Icon::Plus => PLUS,
            Icon::Close => CLOSE,
            Icon::Archive => ARCHIVE,
            Icon::Pin => PIN,
        }
    }
}

pub const PLUS: &[Seg] = &[
    Seg::M(12.0, 5.0),
    Seg::L(12.0, 19.0),
    Seg::M(5.0, 12.0),
    Seg::L(19.0, 12.0),
];
pub const CLOSE: &[Seg] = &[
    Seg::M(18.0, 6.0),
    Seg::L(6.0, 18.0),
    Seg::M(6.0, 6.0),
    Seg::L(18.0, 18.0),
];
pub const ARCHIVE: &[Seg] = &[
    Seg::M(3.0, 3.0),
    Seg::L(21.0, 3.0),
    Seg::C(21.552, 3.0, 22.0, 3.448, 22.0, 4.0),
    Seg::L(22.0, 7.0),
    Seg::C(22.0, 7.552, 21.552, 8.0, 21.0, 8.0),
    Seg::L(3.0, 8.0),
    Seg::C(2.448, 8.0, 2.0, 7.552, 2.0, 7.0),
    Seg::L(2.0, 4.0),
    Seg::C(2.0, 3.448, 2.448, 3.0, 3.0, 3.0),
    Seg::Z,
    Seg::M(4.0, 8.0),
    Seg::L(4.0, 19.0),
    Seg::C(4.0, 20.105, 4.895, 21.0, 6.0, 21.0),
    Seg::L(18.0, 21.0),
    Seg::C(19.105, 21.0, 20.0, 20.105, 20.0, 19.0),
    Seg::L(20.0, 8.0),
    Seg::M(10.0, 12.0),
    Seg::L(14.0, 12.0),
];
pub const PIN: &[Seg] = &[
    Seg::M(12.0, 17.0),
    Seg::L(12.0, 22.0),
    Seg::M(9.0, 10.76),
    Seg::C(9.0, 11.519, 8.57, 12.212, 7.89, 12.55),
    Seg::L(6.11, 13.45),
    Seg::C(5.43, 13.788, 5.0, 14.481, 5.0, 15.24),
    Seg::L(5.0, 16.0),
    Seg::C(5.0, 16.552, 5.448, 17.0, 6.0, 17.0),
    Seg::L(18.0, 17.0),
    Seg::C(18.552, 17.0, 19.0, 16.552, 19.0, 16.0),
    Seg::L(19.0, 15.24),
    Seg::C(19.0, 14.481, 18.57, 13.788, 17.89, 13.45),
    Seg::L(16.11, 12.55),
    Seg::C(15.43, 12.212, 15.0, 11.519, 15.0, 10.76),
    Seg::L(15.0, 7.0),
    Seg::C(15.0, 6.448, 15.448, 6.0, 16.0, 6.0),
    Seg::C(17.105, 6.0, 18.0, 5.105, 18.0, 4.0),
    Seg::C(18.0, 2.895, 17.105, 2.0, 16.0, 2.0),
    Seg::L(8.0, 2.0),
    Seg::C(6.895, 2.0, 6.0, 2.895, 6.0, 4.0),
    Seg::C(6.0, 5.105, 6.895, 6.0, 8.0, 6.0),
    Seg::C(8.552, 6.0, 9.0, 6.448, 9.0, 7.0),
    Seg::Z,
];
