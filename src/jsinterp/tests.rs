//! Upstream `test/test_jsinterp.py`, case for case.
use super::*;

/// `NaN` in upstream's tests: any NaN result.
enum Expected {
    NaN,
    Value(JsValue),
}

fn int(i: i64) -> JsValue {
    JsValue::Int(i)
}

fn s(text: &str) -> JsValue {
    JsValue::str(text)
}

fn list(items: Vec<JsValue>) -> JsValue {
    JsValue::list(items)
}

fn ints(items: &[i64]) -> JsValue {
    list(items.iter().map(|i| int(*i)).collect())
}

fn strs(items: &[&str]) -> JsValue {
    list(items.iter().map(|item| s(item)).collect())
}

fn bools(items: &[bool]) -> JsValue {
    list(items.iter().map(|b| JsValue::Bool(*b)).collect())
}

/// `TestJSInterpreter._test`: `assertEqual` (Python `==`) of the result.
fn check_with(jsi: &JsInterpreter, expected: Expected, func: &str, args: Vec<JsValue>) {
    let got = jsi
        .call_function(func, args)
        .unwrap_or_else(|error| panic!("{}: {error}", jsi.code));
    match expected {
        Expected::NaN => assert!(
            matches!(got, JsValue::Float(f) if f.is_nan()),
            "{got:?} is not NaN"
        ),
        Expected::Value(expected) => assert_eq!(got, expected, "{}", jsi.code),
    }
}

fn check(code: &str, expected: JsValue) {
    check_with(
        &JsInterpreter::new(code),
        Expected::Value(expected),
        "f",
        Vec::new(),
    );
}

fn check_args(code: &str, expected: JsValue, args: Vec<JsValue>) {
    check_with(
        &JsInterpreter::new(code),
        Expected::Value(expected),
        "f",
        args,
    );
}

fn check_nan(code: &str) {
    check_with(&JsInterpreter::new(code), Expected::NaN, "f", Vec::new());
}

#[test]
fn test_basic() {
    let jsi = JsInterpreter::new("function f(){;}");
    assert_eq!(py_repr(&jsi.extract_function("f").unwrap()), "F<f>");
    check_with(&jsi, Expected::Value(JsValue::Null), "f", Vec::new());

    check("function f(){return 42;}", int(42));
    check("function f(){42}", JsValue::Null);
    check("var f = function(){return 42;}", int(42));
}

#[test]
fn test_add() {
    check("function f(){return 42 + 7;}", int(49));
    check_nan("function f(){return 42 + undefined;}");
    check("function f(){return 42 + null;}", int(42));
}

#[test]
fn test_sub() {
    check("function f(){return 42 - 7;}", int(35));
    check_nan("function f(){return 42 - undefined;}");
    check("function f(){return 42 - null;}", int(42));
}

#[test]
fn test_mul() {
    check("function f(){return 42 * 7;}", int(294));
    check_nan("function f(){return 42 * undefined;}");
    check("function f(){return 42 * null;}", int(0));
}

#[test]
fn test_div() {
    let jsi = JsInterpreter::new("function f(a, b){return a / b;}");
    check_with(&jsi, Expected::NaN, "f", vec![int(0), int(0)]);
    check_with(&jsi, Expected::NaN, "f", vec![JsValue::Undefined, int(1)]);
    check_with(
        &jsi,
        Expected::Value(JsValue::Float(f64::INFINITY)),
        "f",
        vec![int(2), int(0)],
    );
    check_with(&jsi, Expected::Value(int(0)), "f", vec![int(0), int(3)]);
}

#[test]
fn test_mod() {
    check("function f(){return 42 % 7;}", int(0));
    check_nan("function f(){return 42 % 0;}");
    check_nan("function f(){return 42 % undefined;}");
}

#[test]
fn test_exp() {
    check("function f(){return 42 ** 2;}", int(1764));
    check_nan("function f(){return 42 ** undefined;}");
    check("function f(){return 42 ** null;}", int(1));
    check_nan("function f(){return undefined ** 42;}");
}

#[test]
fn test_calc() {
    check_args("function f(a){return 2*a+1;}", int(7), vec![int(3)]);
}

#[test]
fn test_empty_return() {
    check("function f(){return; y()}", JsValue::Null);
}

#[test]
fn test_morespace() {
    check_args(
        "function f (a) { return 2 * a + 1 ; }",
        int(7),
        vec![int(3)],
    );
    check("function f () { x =  2  ; return x; }", int(2));
}

#[test]
fn test_strange_chars() {
    check_with(
        &JsInterpreter::new("function $_xY1 ($_axY1) { var $_axY2 = $_axY1 + 1; return $_axY2; }"),
        Expected::Value(int(21)),
        "$_xY1",
        vec![int(20)],
    );
}

#[test]
fn test_operators() {
    check("function f(){return 1 << 5;}", int(32));
    check("function f(){return 2 ** 5}", int(32));
    check("function f(){return 19 & 21;}", int(17));
    check("function f(){return 11 >> 2;}", int(2));
    check("function f(){return []? 2+3: 4;}", int(5));
    check("function f(){return 1 == 2}", JsValue::Bool(false));
    check("function f(){return 0 && 1 || 2;}", int(2));
    check("function f(){return 0 ?? 42;}", int(0));
    check(
        "function f(){return \"life, the universe and everything\" < 42;}",
        JsValue::Bool(false),
    );
    check("function f(){return 0  - 7 * - 6;}", int(42));
    check("function f(){return true << \"5\";}", int(32));
    check("function f(){return true << true;}", int(2));
    check("function f(){return \"19\" & \"21.9\";}", int(17));
    check("function f(){return \"19\" & false;}", int(0));
    check("function f(){return \"11.0\" >> \"2.1\";}", int(2));
    check("function f(){return 5 ^ 9;}", int(12));
    check("function f(){return 0.0 << NaN}", int(0));
    check("function f(){return null << undefined}", int(0));
    check("function f(){return -12616 ^ 5041}", int(-8951));
    check("function f(){return 21 << 4294967297}", int(42));
}

#[test]
fn test_string_concat() {
    check("function f(){return \"a\" + \"b\";}", s("ab"));
    check(
        "function f(){let x = \"a\"; x += \"b\"; return x;}",
        s("ab"),
    );
    check("function f(){return \"a\" + 1;}", s("a1"));
    check("function f(){let x = \"a\"; x += 1; return x;}", s("a1"));
    check("function f(){return 2 + \"b\";}", s("2b"));
    check("function f(){let x = 2; x += \"b\"; return x;}", s("2b"));
}

#[test]
fn test_array_access() {
    check(
        "function f(){var x = [1,2,3]; x[0] = 4; x[0] = 5; x[2.0] = 7; return x;}",
        ints(&[5, 2, 7]),
    );
}

#[test]
fn test_parens() {
    check(
        "function f(){return (1) + (2) * ((( (( (((((3)))))) )) ));}",
        int(7),
    );
    check("function f(){return (1 + 2) * 3;}", int(9));
}

#[test]
fn test_quotes() {
    check(r#"function f(){return "a\"\\("}"#, s(r#"a"\("#));
}

#[test]
fn test_assignments() {
    check("function f(){var x = 20; x = 30 + 1; return x;}", int(31));
    check("function f(){var x = 20; x += 30 + 1; return x;}", int(51));
    check("function f(){var x = 20; x -= 30 + 1; return x;}", int(-11));
    check(
        "function f(){var x = 2; var y = [\"a\", \"b\"]; y[x%y[\"length\"]]=\"z\"; return y}",
        strs(&["z", "b"]),
    );
}

#[test]
#[ignore = "upstream: @unittest.skip('Not implemented')"]
fn test_comments() {
    check(
        "
            function f() {
                var x = /* 1 + */ 2;
                var y = /* 30
                * 40 */ 50;
                return x + y;
            }
        ",
        int(52),
    );
    check(
        "
            function f() {
                var x = \"/*\";
                var y = 1 /* comment */ + 2;
                return y;
            }
        ",
        int(3),
    );
}

#[test]
fn test_precedence() {
    check(
        "
            function f() {
                var a = [10, 20, 30, 40, 50];
                var b = 6;
                a[0]=a[b%a.length];
                return a;
            }
        ",
        ints(&[20, 20, 30, 40, 50]),
    );
}

#[test]
fn test_builtins() {
    check_nan("function f() { return NaN }");
}

#[test]
fn test_date() {
    check(
        "function f() { return new Date(\"Wednesday 31 December 1969 18:01:26 MDT\") - 0; }",
        int(86000),
    );
    let jsi = JsInterpreter::new("function f(dt) { return new Date(dt) - 0; }");
    for (date, expected) in [
        ("Wednesday 31 December 1969 18:01:26 MDT", 86000),
        ("12/31/1969 18:01:26 MDT", 86000), // m/d/y
        ("1 January 1970 00:00:00 UTC", 0),
    ] {
        check_with(&jsi, Expected::Value(int(expected)), "f", vec![s(date)]);
    }
}

#[test]
fn test_call() {
    let jsi = JsInterpreter::new(
        "
            function x() { return 2; }
            function y(a) { return x() + (a?a:0); }
            function z() { return y(3); }
        ",
    );
    check_with(&jsi, Expected::Value(int(5)), "z", Vec::new());
    check_with(&jsi, Expected::Value(int(2)), "y", Vec::new());
}

#[test]
fn test_if() {
    check(
        "
            function f() {
                let a = 9;
                if (0==0) {a++}
                return a
            }
        ",
        int(10),
    );
    check(
        "
            function f() {
                if (0==0) {return 10}
            }
        ",
        int(10),
    );
    check(
        "
            function f() {
                if (0!=0) {return 1}
                else {return 10}
            }
        ",
        int(10),
    );
    // Upstream leaves `else if` commented out as unsupported.
}

#[test]
fn test_for_loop() {
    check(
        "function f() { a=0; for (i=0; i-10; i++) {a++} return a }",
        int(10),
    );
}

#[test]
fn test_switch() {
    let jsi = JsInterpreter::new(
        "
            function f(x) { switch(x){
                case 1:x+=1;
                case 2:x+=2;
                case 3:x+=3;break;
                case 4:x+=4;
                default:x=0;
            } return x }
        ",
    );
    check_with(&jsi, Expected::Value(int(7)), "f", vec![int(1)]);
    check_with(&jsi, Expected::Value(int(6)), "f", vec![int(3)]);
    check_with(&jsi, Expected::Value(int(0)), "f", vec![int(5)]);
}

#[test]
fn test_switch_default() {
    let jsi = JsInterpreter::new(
        "
            function f(x) { switch(x){
                case 2: x+=2;
                default: x-=1;
                case 5:
                case 6: x+=6;
                case 0: break;
                case 1: x+=1;
            } return x }
        ",
    );
    check_with(&jsi, Expected::Value(int(2)), "f", vec![int(1)]);
    check_with(&jsi, Expected::Value(int(11)), "f", vec![int(5)]);
    check_with(&jsi, Expected::Value(int(14)), "f", vec![int(9)]);
}

#[test]
fn test_try() {
    check(
        "function f() { try{return 10} catch(e){return 5} }",
        int(10),
    );
}

#[test]
fn test_catch() {
    check("function f() { try{throw 10} catch(e){return 5} }", int(5));
}

#[test]
fn test_finally() {
    check(
        "function f() { try{throw 10} finally {return 42} }",
        int(42),
    );
    check(
        "function f() { try{throw 10} catch(e){return 5} finally {return 42} }",
        int(42),
    );
}

#[test]
fn test_nested_try() {
    check(
        "
            function f() {try {
                try{throw 10} finally {throw 42}
                } catch(e){return 5} }
        ",
        int(5),
    );
}

#[test]
fn test_for_loop_continue() {
    check(
        "function f() { a=0; for (i=0; i-10; i++) { continue; a++ } return a }",
        int(0),
    );
}

#[test]
fn test_for_loop_break() {
    check(
        "function f() { a=0; for (i=0; i-10; i++) { break; a++ } return a }",
        int(0),
    );
}

#[test]
fn test_for_loop_try() {
    check(
        "
            function f() {
                for (i=0; i-10; i++) { try { if (i == 5) throw i} catch {return 10} finally {break} };
                return 42 }
        ",
        int(42),
    );
}

#[test]
fn test_literal_list() {
    check(
        "function f() { return [1, 2, \"asdf\", [5, 6, 7]][3] }",
        ints(&[5, 6, 7]),
    );
}

#[test]
fn test_comma() {
    check("function f() { a=5; a -= 1, a+=3; return a }", int(7));
    check("function f() { a=5; return (a -= 1, a+=3, a); }", int(7));
    check(
        "function f() { return (l=[0,1,2,3], function(a, b){return a+b})((l[1], l[2]), l[3]) }",
        int(5),
    );
}

#[test]
fn test_void() {
    check("function f() { return void 42; }", JsValue::Null);
}

#[test]
fn test_return_function() {
    let jsi = JsInterpreter::new(
        "\n            function f() { return [1, function(){return 1}][1] }\n        ",
    );
    let function = jsi.call_function("f", Vec::new()).unwrap();
    assert_eq!(jsi.call(&function, Vec::new()).unwrap(), int(1));
}

#[test]
fn test_null() {
    check("function f() { return null; }", JsValue::Null);
    check(
        "function f() { return [null > 0, null < 0, null == 0, null === 0]; }",
        bools(&[false, false, false, false]),
    );
    check(
        "function f() { return [null >= 0, null <= 0]; }",
        bools(&[true, true]),
    );
}

#[test]
fn test_undefined() {
    check(
        "function f() { return undefined === undefined; }",
        JsValue::Bool(true),
    );
    check("function f() { return undefined; }", JsValue::Undefined);
    check("function f() {return undefined ?? 42; }", int(42));
    check("function f() { let v; return v; }", JsValue::Undefined);
    check("function f() { let v; return v**0; }", int(1));
    check(
        "function f() { let v; return [v>42, v<=42, v&&42, 42&&v]; }",
        list(vec![
            JsValue::Bool(false),
            JsValue::Bool(false),
            JsValue::Undefined,
            JsValue::Undefined,
        ]),
    );
    check(
        "
            function f() { return [
                undefined === undefined,
                undefined == undefined,
                undefined == null,
                undefined < undefined,
                undefined > undefined,
                undefined === 0,
                undefined == 0,
                undefined < 0,
                undefined > 0,
                undefined >= 0,
                undefined <= 0,
                undefined > null,
                undefined < null,
                undefined === null
            ]; }
        ",
        bools(&[
            true, true, true, false, false, false, false, false, false, false, false, false, false,
            false,
        ]),
    );

    let jsi = JsInterpreter::new(
        "\n            function f() { let v; return [42+v, v+42, v**42, 42**v, 0**v]; }\n        ",
    );
    let JsValue::List(items) = jsi.call_function("f", Vec::new()).unwrap() else {
        panic!("not a list");
    };
    for item in items.borrow().iter() {
        assert!(matches!(item, JsValue::Float(f) if f.is_nan()), "{item:?}");
    }
}

#[test]
fn test_object() {
    check("function f() { return {}; }", JsValue::object::<&str>([]));
    check(
        "function f() { let a = {m1: 42, m2: 0 }; return [a[\"m1\"], a.m2]; }",
        ints(&[42, 0]),
    );
    check("function f() { let a; return a?.qq; }", JsValue::Undefined);
    check(
        "function f() { let a = {m1: 42, m2: 0 }; return a?.qq; }",
        JsValue::Undefined,
    );
    check(
        "function f() { let a = {\"1\": 123}; return a[1]; }",
        int(123),
    );
}

#[test]
fn test_regex() {
    check("function f() { let a=/,,[/,913,/](,)}/; }", JsValue::Null);
    check(
        "function f() { let a=/,,[/,913,/](,)}/; return a; }",
        s("/,,[/,913,/](,)}/0"),
    );
    // Upstream's compiled-pattern cases are commented out: it does not
    // compile regex literals.
}

#[test]
#[ignore = "upstream: @unittest.skip('Not implemented')"]
fn test_replace() {
    check(
        "function f() { let a=\"data-name\".replace(\"data-\", \"\"); return a }",
        s("name"),
    );
    check(
        "function f() { let a=\"data-name\".replace(new RegExp(\"^.+-\"), \"\"); return a; }",
        s("name"),
    );
    check(
        "function f() { let a=\"data-name\".replace(/^.+-/, \"\"); return a; }",
        s("name"),
    );
    check(
        "function f() { let a=\"data-name\".replace(/a/g, \"o\"); return a; }",
        s("doto-nome"),
    );
    check(
        "function f() { let a=\"data-name\".replaceAll(\"a\", \"o\"); return a; }",
        s("doto-nome"),
    );
}

#[test]
fn test_char_code_at() {
    let jsi = JsInterpreter::new("function f(i){return \"test\".charCodeAt(i)}");
    for (arg, expected) in [
        (int(0), int(116)),
        (int(1), int(101)),
        (int(2), int(115)),
        (int(3), int(116)),
        (int(4), JsValue::Null),
        (s("not_a_number"), int(116)),
    ] {
        check_with(&jsi, Expected::Value(expected), "f", vec![arg]);
    }
}

#[test]
fn test_bitwise_operators_overflow() {
    check("function f(){return -524999584 << 5}", int(379882496));
    check("function f(){return 1236566549 << 5}", int(915423904));
}

#[test]
fn test_bitwise_operators_typecast() {
    check("function f(){return null << 5}", int(0));
    check("function f(){return undefined >> 5}", int(0));
    check("function f(){return 42 << NaN}", int(42));
}

#[test]
fn test_negative() {
    check("function f(){return 2    *    -2.0    ;}", int(-4));
    check("function f(){return 2    -    - -2    ;}", int(0));
    check("function f(){return 2    -    - - -2  ;}", int(4));
    check("function f(){return 2    -    + + - -2;}", int(0));
    check("function f(){return 2    +    - + - -2;}", int(0));
}

#[test]
#[ignore = "upstream: @unittest.skip('Not implemented')"]
fn test_packed() {
    // Upstream's packed (`p,a,c,k,e,d`) case needs `String.replace` and
    // `new RegExp`, which it does not implement either.
    let jsi = JsInterpreter::new(
        r"function f(p,a,c,k,e,d){while(c--)if(k[c])p=p.replace(new RegExp('\\b'+c.toString(a)+'\\b','g'),k[c]);return p}",
    );
    jsi.call_function("f", Vec::new()).unwrap();
}

#[test]
fn test_join() {
    let test_input = || strs(&["t", "e", "s", "t"]);
    for code in [
        "function f(a, b){return a.join(b)}",
        "function f(a, b){return Array.prototype.join.call(a, b)}",
        "function f(a, b){return Array.prototype.join.apply(a, [b])}",
    ] {
        let jsi = JsInterpreter::new(code);
        check_with(
            &jsi,
            Expected::Value(s("test")),
            "f",
            vec![test_input(), s("")],
        );
        check_with(
            &jsi,
            Expected::Value(s("t-e-s-t")),
            "f",
            vec![test_input(), s("-")],
        );
        check_with(
            &jsi,
            Expected::Value(s("")),
            "f",
            vec![list(Vec::new()), s("-")],
        );
    }
}

#[test]
fn test_split() {
    let test_result = || strs(&["t", "e", "s", "t"]);
    for code in [
        "function f(a, b){return a.split(b)}",
        "function f(a, b){return a[\"split\"](b)}",
        "function f(a, b){let x = [\"split\"]; return a[x[0]](b)}",
        "function f(a, b){return String.prototype.split.call(a, b)}",
        "function f(a, b){return String.prototype.split.apply(a, [b])}",
    ] {
        let jsi = JsInterpreter::new(code);
        check_with(
            &jsi,
            Expected::Value(test_result()),
            "f",
            vec![s("test"), s("")],
        );
        check_with(
            &jsi,
            Expected::Value(test_result()),
            "f",
            vec![s("t-e-s-t"), s("-")],
        );
        check_with(&jsi, Expected::Value(strs(&[""])), "f", vec![s(""), s("-")]);
        check_with(
            &jsi,
            Expected::Value(list(Vec::new())),
            "f",
            vec![s(""), s("")],
        );
    }
}

#[test]
fn test_slice() {
    let all = [0, 1, 2, 3, 4, 5, 6, 7, 8];
    for (args, expected) in [
        ("", &all[..]),
        ("0", &all[..]),
        ("5", &all[5..]),
        ("99", &[][..]),
        ("-2", &all[7..]),
        ("-99", &all[..]),
        ("0, 0", &[][..]),
        ("1, 0", &[][..]),
        ("0, 1", &all[..1]),
        ("3, 6", &all[3..6]),
        ("1, -1", &all[1..8]),
        ("-1, 1", &[][..]),
        ("-3, -1", &all[6..8]),
    ] {
        check(
            &format!("function f(){{return [0, 1, 2, 3, 4, 5, 6, 7, 8].slice({args})}}"),
            ints(expected),
        );
    }
    for (args, expected) in [
        ("", "012345678"),
        ("0", "012345678"),
        ("5", "5678"),
        ("99", ""),
        ("-2", "78"),
        ("-99", "012345678"),
        ("0, 0", ""),
        ("1, 0", ""),
        ("0, 1", "0"),
        ("3, 6", "345"),
        ("1, -1", "1234567"),
        ("-1, 1", ""),
        ("-3, -1", "67"),
    ] {
        check(
            &format!("function f(){{return \"012345678\".slice({args})}}"),
            s(expected),
        );
    }
}

#[test]
fn test_splice() {
    check(
        "function f(){var T = [\"0\", \"1\", \"2\"]; T[\"splice\"](2, 1, \"0\")[0]; return T }",
        strs(&["0", "1", "0"]),
    );
}

#[test]
fn test_int_to_int32() {
    for (input, expected) in [
        (0, 0),
        (1, 1),
        (-1, -1),
        (-8951, -8951),
        (2147483647, 2147483647),
        (2147483648, -2147483648),
        (2147483649, -2147483647),
        (-2147483649, 2147483647),
        (-2147483648, -2147483648),
        (-16799986688, 379882496),
        (39570129568, 915423904),
    ] {
        assert_eq!(int_to_int32(input), expected, "{input}");
    }
}

#[test]
fn test_js_number_to_string() {
    for (value, radix, expected) in [
        (0.0, None, "0"),
        (-0.0, None, "0"),
        (f64::NAN, None, "NaN"),
        (-f64::NAN, None, "NaN"),
        (f64::INFINITY, None, "Infinity"),
        (f64::NEG_INFINITY, None, "-Infinity"),
        (10f64.powf(21.5), Some(8), "526665530627250154000000"),
        (6.0, Some(2), "110"),
        (254.0, Some(16), "fe"),
        (-10.0, Some(2), "-1010"),
        (-255.0, Some(2), "-11111111"),
        (0.1 + 0.2, Some(16), "0.4cccccccccccd"),
        (1234.1234, Some(10), "1234.1234"),
        // Upstream comments out (1000000000000000128, 10, '1000000000000000100').
    ] {
        assert_eq!(
            js_number_to_string(value, radix),
            expected,
            "{value} {radix:?}"
        );
    }
}

#[test]
fn test_extract_function() {
    let jsi = JsInterpreter::new("function a(b) { return b + 1; }");
    let function = jsi.extract_function("a").unwrap();
    assert_eq!(jsi.call(&function, vec![int(2)]).unwrap(), int(3));
}

#[test]
fn test_extract_function_with_global_stack() {
    let jsi = JsInterpreter::new("function c(d) { return d + e + f + g; }");
    let function = jsi
        .extract_function_with_globals(
            "c",
            vec![
                scope([("e", int(10))]),
                scope([("f", int(100)), ("g", int(1000))]),
            ],
        )
        .unwrap();
    assert_eq!(jsi.call(&function, vec![int(1)]).unwrap(), int(1111));
}

#[test]
fn test_extract_object() {
    let jsi =
        JsInterpreter::new("var a={};a.xy={};var xy;var zxy={};xy={z:function(){return \"abc\"}};");
    // Upstream passes a `None` global stack, which is never used.
    let JsValue::Object(object) = jsi.extract_object("xy", &[]).unwrap() else {
        panic!("not an object");
    };
    assert!(object_get(&object.borrow(), &s("z")).is_some());
}

#[test]
fn test_increment_decrement() {
    check("function f() { var x = 1; return ++x; }", int(2));
    check("function f() { var x = 1; return x++; }", int(1));
    check("function f() { var x = 1; x--; return x }", int(0));
    check(
        "function f() { var y; var x = 1; x++, --x, x--, x--, y=\"z\", \"abc\", x++; return --x }",
        int(-1),
    );
    check(
        "function f() { var a = \"test--\"; return a; }",
        s("test--"),
    );
    check(
        "function f() { var b = 1; var a = \"b--\"; return a; }",
        s("b--"),
    );
}

#[test]
fn test_nested_function_scoping() {
    check(
        r"
            function f() {
                var g = function() {
                    var P = 2;
                    return P;
                };
                var P = 1;
                g();
                return P;
            }
        ",
        int(1),
    );
    check(
        r#"
            function f() {
                var x = function() {
                    for (var w = 1, M = []; w < 2; w++) switch (w) {
                        case 1:
                            M.push("a");
                        case 2:
                            M.push("b");
                    }
                    return M
                };
                var w = "c";
                var M = "d";
                var y = x();
                y.push(w);
                y.push(M);
                return y;
            }
        "#,
        strs(&["a", "b", "c", "d"]),
    );
    check(
        r"
            function f() {
                var P, Q;
                var z = 100;
                var g = function() {
                    var P, Q; P = 2; Q = 15;
                    z = 0;
                    return P+Q;
                };
                P = 1; Q = 10;
                var x = g(), y = 3;
                return P+Q+x+y+z;
            }
        ",
        int(31),
    );
}

#[test]
fn test_undefined_varnames() {
    let jsi = JsInterpreter::new("function f(){ var a; return [a, b]; }");
    check_with(
        &jsi,
        Expected::Value(list(vec![JsValue::Undefined, JsValue::Undefined])),
        "f",
        Vec::new(),
    );
    assert_eq!(
        jsi.undefined_varnames().into_iter().collect::<Vec<_>>(),
        ["b"]
    );
}

// Upstream error messages and quirks this port keeps (not in upstream's
// test file).

fn error(code: &str) -> String {
    JsInterpreter::new(code)
        .call_function("f", Vec::new())
        .unwrap_err()
        .orig_msg()
}

#[test]
fn reports_upstream_errors() {
    assert_eq!(
        JsInterpreter::new("var x = 1;")
            .call_function("f", Vec::new())
            .unwrap_err()
            .orig_msg(),
        "Could not find JS function \"f\""
    );
    assert_eq!(
        error("function f(){ return 1 @ 2; }"),
        "Unsupported JS expression 1 @ 2 in: return 1 @ 2"
    );
    assert_eq!(
        error("function f(){ return 1 - \"a\"; }"),
        "Failed to evaluate 1 - 'a' in: 1 - \"a\""
    );
    assert_eq!(
        error("function f(){ return x.y(); }"),
        "Could not find object x"
    );
    assert_eq!(
        error("function f(){ return Math.floor(1); }"),
        "Unsupported Math method floor in: Math.floor(1)"
    );
    assert_eq!(
        error("function f(){ return String.raw(1); }"),
        "Unsupported String method raw in: String.raw(1)"
    );
    assert_eq!(
        error("function f(){ return new Foo(1); }"),
        "Unsupported object Foo(1) in: new Foo(1)"
    );
    assert_eq!(error("function f(){ throw 5; }"), "Uncaught exception 5");
    assert_eq!(error("function f(){ break; }"), "Invalid break");
    assert_eq!(
        error("function f(){ var x = [1]; return x[5]; }"),
        "Cannot get index 5 in: [1]"
    );
    assert_eq!(
        error("function f(){ return [1].join(\"\", 2); }"),
        "join takes exactly one argument in: __yt_dlp_jsinterp_obj1.join(\"\", 2)"
    );
    // No space is allowed between `catch (e)` and `{`.
    assert_eq!(
        error("function f(){ try { throw 1 } catch (e) { return e } }"),
        "Uncaught exception 1"
    );
    let full = JsInterpreter::new("function f(){ throw 5; }")
        .call_function("f", Vec::new())
        .unwrap_err()
        .to_string();
    assert!(full.starts_with("Uncaught exception 5; please report this issue on"));
}

#[test]
fn keeps_python_semantics() {
    // A leading `-` splits first: `None - (7 % 3)`.
    check("function f(){ return -7 % 3; }", int(-1));
    check("function f(){ var a = -7; return a % 3; }", int(2));
    check("function f(){ return 7 / 2; }", JsValue::Float(3.5));
    check(
        "function f(){ return 1 / 0; }",
        JsValue::Float(f64::INFINITY),
    );
    check(
        "function f(){ return -1 / 0; }",
        JsValue::Float(f64::NEG_INFINITY),
    );
    // `(e)` (with its parentheses) is the name `catch (e)` binds.
    let jsi = JsInterpreter::new("function f(){ try { throw 1 } catch (e){ return e } }");
    check_with(&jsi, Expected::Value(JsValue::Undefined), "f", Vec::new());
    assert_eq!(
        jsi.undefined_varnames().into_iter().collect::<Vec<_>>(),
        ["e"]
    );
    check("function f(){ return \"a\" + 1.0; }", s("a1.0"));
    check("function f(){ return \"a\" + true; }", s("aTrue"));
    check("function f(){ return [1].push(2); }", ints(&[1, 2]));
    check(
        "function f(){ return String.fromCharCode(72, 105); }",
        s("Hi"),
    );
    check("function f(){ return Math.pow(2, 10); }", int(1024));
    check("function f(){ return \"abc\".indexOf(\"c\"); }", int(2));
    check("function f(){ return [3, 4].indexOf(5); }", int(-1));
    check("function f(){ return /a/g; }", s("/a/2048"));
    check(
        "function f(){ var a = [1, 2, 3]; a.reverse(); a.unshift(0); return a; }",
        ints(&[0, 3, 2, 1]),
    );
}

#[test]
fn reaches_the_recursion_limit() {
    let nested = format!(
        "function f(){{ return {}1{}; }}",
        "(".repeat(120),
        ")".repeat(120)
    );
    assert_eq!(error(&nested), "Recursion limit reached");
}

/// The class name upstream reports for an error.
fn class_name(error: &JsError) -> &'static str {
    match error {
        JsError::Exception { .. } => "Exception",
        JsError::Break => "JS_Break",
        JsError::Continue => "JS_Continue",
        JsError::Throw(_) => "JS_Throw",
        JsError::Python { kind, .. } => kind,
    }
}

/// tests/fixtures/jsinterp: pinned upstream results of
/// `JSInterpreter(code).call_function(func, *args)`, saved by
/// `tools/capture_jsinterp.py`. Python's own exceptions (`TypeError` and the
/// like) are compared by class, the interpreter's by message too.
#[test]
fn matches_saved_upstream_results() {
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/jsinterp/expected.json")).unwrap();
    let mut failures = Vec::new();
    for case in expected["cases"].as_array().unwrap() {
        // Documented differences (the case says why).
        if case.get("divergence").is_some() {
            continue;
        }
        let code = case["code"].as_str().unwrap();
        let func = case["func"].as_str().unwrap_or("f");
        let args = case["args"]
            .as_array()
            .map(|args| args.iter().cloned().map(value::from_json).collect())
            .unwrap_or_default();
        let got = JsInterpreter::new(code).call_function(func, args);
        let wanted = match case["result"].as_str() {
            Some(result) => result.to_owned(),
            None => format!(
                "{}: {}",
                case["error"].as_str().unwrap(),
                case["message"].as_str().unwrap()
            ),
        };
        let matched = match (&got, case["result"].as_str()) {
            (Ok(value), Some(result)) => py_repr(value) == result,
            (Err(error @ JsError::Python { .. }), None) => {
                Some(class_name(error)) == case["error"].as_str()
            }
            (Err(error), None) => format!("{}: {}", class_name(error), error.orig_msg()) == wanted,
            _ => false,
        };
        if !matched {
            let got = match &got {
                Ok(value) => py_repr(value),
                Err(error) => format!("{}: {}", class_name(error), error.orig_msg()),
            };
            failures.push(format!("{code}\n  upstream: {wanted}\n  got:      {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
