// Tests for `number_to_string`: ECMAScript `Number#toString`; expectations are from node 24.4, `String(x)`.
// `numericMangling` rows (holesail-cli/keyderivation-vectors.json) - the key after minimist.
use super::*;
use townjump_testkit::vectors::load_value;

mod integers {
    use super::*;

    #[test]
    fn are_written_without_fraction_and_exponent() {
        check(INTEGERS);
    }

    #[test]
    fn negative_zero_is_zero() {
        // node 24.4, String(-0)
        assert_eq!(number_to_string(-0.0), "0");
    }
}

mod fractions {
    use super::*;

    #[test]
    fn use_shortest_round_trip_digits() {
        check(FRACTIONS);
    }

    #[test]
    fn small_values_switch_to_exponent_below_1e_6() {
        check(SMALL);
    }
}

mod large_values {
    use super::*;

    #[test]
    fn stay_decimal_below_1e21_and_use_exponent_from_it() {
        check(LARGE);
    }
}

mod specials {
    use super::*;

    #[test]
    fn nan_is_nan() {
        assert_eq!(number_to_string(f64::NAN), "NaN");
    }

    #[test]
    fn infinities_carry_sign() {
        assert_eq!(number_to_string(f64::INFINITY), "Infinity");
        assert_eq!(number_to_string(f64::NEG_INFINITY), "-Infinity");
    }
}

mod minimist_numeric_keys {
    use super::*;

    #[test]
    fn argv_text_becomes_the_key_from_the_vector() {
        let file = load_value("holesail-cli/keyderivation-vectors.json");
        for row in file["numericMangling"].as_array().expect("numericMangling") {
            let text = row["argvText"].as_str().expect("argvText");
            let want = row["asKey"].as_str().expect("asKey");
            let n: f64 = text.parse().expect("number-like string");
            assert_eq!(number_to_string(n), want, "argvText {text:?}");
        }
    }
}

// node 24.4, String(x)
const INTEGERS: &[(f64, &str)] = &[
    (0.0, "0"),
    (1.0, "1"),
    (-1.0, "-1"),
    (10.0, "10"),
    (16.0, "16"),
    (3000.0, "3000"),
    (65535.0, "65535"),
    (65536.0, "65536"),
    (99999.0, "99999"),
    (123456.0, "123456"),
    (4294967295.0, "4294967295"),
    (4294967296.0, "4294967296"),
    (9007199254740991.0, "9007199254740991"),
    (9007199254740992.0, "9007199254740992"),
    (1000000000000000.0, "1000000000000000"),
    (123456789012345680.0, "123456789012345680"),
    (-9007199254740991.0, "-9007199254740991"),
];

const FRACTIONS: &[(f64, &str)] = &[
    (3000.5, "3000.5"),
    (-3000.5, "-3000.5"),
    (0.5, "0.5"),
    (0.1, "0.1"),
    (-0.1, "-0.1"),
    (1.5, "1.5"),
    (0.30000000000000004, "0.30000000000000004"),
    (0.7999999999999999, "0.7999999999999999"),
    (0.3333333333333333, "0.3333333333333333"),
    (123.456, "123.456"),
    (12345.6789, "12345.6789"),
    (4.35, "4.35"),
    (0.145, "0.145"),
    (123456789.12345679, "123456789.12345679"),
];

const SMALL: &[(f64, &str)] = &[
    (0.000123, "0.000123"),
    (0.000001, "0.000001"),
    (0.0000015, "0.0000015"),
    (0.0000123, "0.0000123"),
    (1e-7, "1e-7"),
    (1.5e-7, "1.5e-7"),
    (5e-7, "5e-7"),
    (1e-10, "1e-10"),
    (1.23e-18, "1.23e-18"),
    (1e-320, "1e-320"),
    (5e-324, "5e-324"),
    (-1e-7, "-1e-7"),
    (-1.5e-7, "-1.5e-7"),
    (0.000001234, "0.000001234"),
    (2.2250738585072014e-308, "2.2250738585072014e-308"),
];

const LARGE: &[(f64, &str)] = &[
    (1e20, "100000000000000000000"),
    (1.5e20, "150000000000000000000"),
    (123456789012345680000.0, "123456789012345680000"),
    (999999999999999900000.0, "999999999999999900000"),
    (1e21, "1e+21"),
    (1.5e21, "1.5e+21"),
    (1e22, "1e+22"),
    (1.2345678901234568e21, "1.2345678901234568e+21"),
    (1e100, "1e+100"),
    (1.234567890123457e31, "1.234567890123457e+31"),
    (1.2345678901234568e38, "1.2345678901234568e+38"),
    (1e300, "1e+300"),
    (-1e21, "-1e+21"),
    (f64::MAX, "1.7976931348623157e+308"),
    (f64::MIN, "-1.7976931348623157e+308"),
];

fn check(table: &[(f64, &str)]) {
    for (n, want) in table {
        assert_eq!(number_to_string(*n), *want, "String({n:?})");
    }
}
