wit_bindgen::generate!({
    world: "js-component",
    path: "wit",
});

use exports::test::js_component::tools;

struct Component;

impl Guest for Component {
    fn echo_bytes(b: Vec<u8>) -> Vec<u8> {
        b
    }

    fn wide(a: u64, b: i64) -> (u64, i64) {
        (a, b)
    }

    fn nest(x: Option<Option<u32>>) -> Option<Option<u32>> {
        x
    }

    fn describe(s: ShapeInfo) -> ShapeInfo {
        ShapeInfo {
            fill_color: match s.fill_color {
                Color::Red => Color::DarkBlue,
                Color::DarkBlue => Color::Red,
            },
            perms: s.perms | Perms::WRITE_ALL,
            shape: match s.shape {
                Shape::Empty => Shape::Circle(1.5),
                Shape::Circle(radius) => Shape::Circle(radius * 2.0),
            },
        }
    }

    fn parse(s: String) -> Result<u32, ParseError> {
        s.parse()
            .map_err(|error: std::num::ParseIntError| ParseError {
                code: 7,
                message: error.to_string(),
            })
    }

    fn spin() {
        loop {
            std::hint::black_box(());
        }
    }
}

impl tools::Guest for Component {
    fn frob(x: u64) -> u64 {
        x.wrapping_mul(3)
    }
}

export!(Component);
