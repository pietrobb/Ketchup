//! Rust math on the exact path goes through crmath (third_party/crmath), the
//! correctly rounded libm that build.rs links into every binary using this
//! crate. Correct rounding fixes every result bit, so these digests are the
//! same on macOS and Windows; with a platform libm they are not (Apple's and
//! Microsoft's sin, pow, ... differ in the last bit), and documents pin exact
//! results bit for bit.

use std::hint::black_box;

const SAMPLES: usize = 20_000;

struct Inputs(u64);

impl Inputs {
    fn bits(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Small, CAD-sized, large and arbitrary-bit arguments in turn.
    fn next(&mut self, index: usize) -> f64 {
        let unit = (self.bits() >> 11) as f64 * f64::EPSILON / 2.0;
        match index % 4 {
            0 => (unit * 2.0 - 1.0) * 8.0,
            1 => unit * 1e3,
            2 => f64::from_bits(self.bits()),
            _ => (unit * 2.0 - 1.0) * 1e6,
        }
    }
}

fn digest(results: impl Iterator<Item = u64>) -> u64 {
    results.fold(0xcbf2_9ce4_8422_2325, |hash, bits| {
        bits.to_le_bytes().iter().fold(hash, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    })
}

/// The result's bits; every NaN counts as one value, since NaN payloads are
/// not specified.
fn canonical(result: f64) -> u64 {
    if result.is_nan() {
        u64::MAX
    } else {
        result.to_bits()
    }
}

fn unary(function: fn(f64) -> f64) -> u64 {
    let mut inputs = Inputs(0x9e37_79b9_7f4a_7c15);
    digest((0..SAMPLES).map(|index| canonical(function(black_box(inputs.next(index))))))
}

fn binary(function: fn(f64, f64) -> f64) -> u64 {
    let mut inputs = Inputs(0x2545_f491_4f6c_dd1d);
    digest((0..SAMPLES).map(|index| {
        let x = inputs.next(index);
        let y = inputs.next(index + 1);
        canonical(function(black_box(x), black_box(y)))
    }))
}

fn single(function: fn(f32, f32) -> f32) -> u64 {
    let mut inputs = Inputs(0x8000_0000_0000_0001);
    digest((0..SAMPLES).map(|index| {
        let x = inputs.next(index) as f32;
        let y = (inputs.next(index + 3) as f32).clamp(-40.0, 40.0);
        canonical(f64::from(function(black_box(x), black_box(y))))
    }))
}

#[test]
fn libm_results_are_correctly_rounded_and_platform_independent() {
    // Linking the crate pulls crmath into this binary.
    let _ = ketchup_exact::backend_fingerprint();
    // f64::atanh is absent: Rust computes it from ln_1p and basic arithmetic
    // instead of calling libm, which is platform independent but not
    // correctly rounded.
    let actual = [
        ("sin", unary(f64::sin)),
        ("cos", unary(f64::cos)),
        ("tan", unary(f64::tan)),
        ("asin", unary(f64::asin)),
        ("acos", unary(f64::acos)),
        ("atan", unary(f64::atan)),
        ("sinh", unary(f64::sinh)),
        ("cosh", unary(f64::cosh)),
        ("tanh", unary(f64::tanh)),
        ("asinh", unary(f64::asinh)),
        ("acosh", unary(f64::acosh)),
        ("exp", unary(f64::exp)),
        ("exp2", unary(f64::exp2)),
        ("ln", unary(f64::ln)),
        ("log2", unary(f64::log2)),
        ("log10", unary(f64::log10)),
        ("ln_1p", unary(f64::ln_1p)),
        ("cbrt", unary(f64::cbrt)),
        ("atan2", binary(f64::atan2)),
        ("powf", binary(f64::powf)),
        ("hypot", binary(f64::hypot)),
        ("f32 acos", single(|x, _| x.acos())),
        ("f32 tan", single(|x, _| x.tan())),
        ("f32 powf", single(f32::powf)),
    ];
    let expected: [(&str, u64); 24] = EXPECTED;
    let mismatches: Vec<String> = actual
        .iter()
        .zip(expected)
        .filter(|((_, got), (_, want))| got != want)
        .map(|((name, got), (_, want))| format!("{name}: {got:#018x}, expected {want:#018x}"))
        .collect();
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// Digests of the correctly rounded results, computed independently with MPFR
/// (precision 53 or 24, round to nearest, subnormals emulated) from the same
/// inputs: these are the true values, not one platform's answers.
const EXPECTED: [(&str, u64); 24] = [
    ("sin", 0x23c4d927422b2db0),
    ("cos", 0xb6711b551df67d50),
    ("tan", 0xc8fe5f3ec5351d68),
    ("asin", 0xaceddf8e2339002f),
    ("acos", 0x99df96d37e8369e5),
    ("atan", 0xc302591285a6233e),
    ("sinh", 0x7fbb99df06a839da),
    ("cosh", 0x36fea4c913d2c4f2),
    ("tanh", 0xaf187c807b646e51),
    ("asinh", 0x8e59e82ce05893f5),
    ("acosh", 0xa3eec60c9216af89),
    ("exp", 0x731230ae9c2209c2),
    ("exp2", 0xca90d2af142d9009),
    ("ln", 0xe565884c99d9d87d),
    ("log2", 0x940f6bf872d87703),
    ("log10", 0x02085c1cbf7bc985),
    ("ln_1p", 0x2b349e8a999b9541),
    ("cbrt", 0x1690388f7da03031),
    ("atan2", 0x856a2407ebf36389),
    ("powf", 0xa284b37584881b35),
    ("hypot", 0x6ebc84e96a83f2dd),
    ("f32 acos", 0xe40238e63219eab3),
    ("f32 tan", 0x8ab821989c03a878),
    ("f32 powf", 0xdddd95210fba5074),
];
