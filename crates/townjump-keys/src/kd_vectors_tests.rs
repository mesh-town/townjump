// Keys test helpers: records of `holesail/key-derivation.json` (84 records, 10 errors), rows of `holesail-cli/keyderivation-vectors.json`
// and the randomness `00 01 … 1f`.
use serde::Deserialize;
use serde_json::Value;
use std::cell::Cell;
use townjump_testkit::vectors::{load, load_value};

#[derive(Deserialize)]
struct File {
    vectors: Vec<Rec>,
}

#[derive(Deserialize)]
pub struct Rec {
    pub input: Input,
    pub holesail: Option<Hs>,
    pub result: Option<Value>,
    pub error: Option<Value>,
}

#[derive(Deserialize)]
pub struct Input {
    pub role: String,
    pub key: Typed,
    pub secure: Typed,
}

#[derive(Deserialize)]
pub struct Typed {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: Option<Value>,
}

/// Holesail state after `urlParser` and `#initialise`.
#[derive(Deserialize)]
pub struct Hs {
    pub key: Option<Value>,
    pub secure: Option<Value>,
}

impl Rec {
    /// `key` argument for `server_keys`/`client_identity`: empty if no key is set.
    pub fn key_arg(&self) -> Option<String> {
        if self.input.key.kind == "undefined" {
            return None;
        }
        Some(self.hs_key()).filter(|k| !k.is_empty())
    }

    /// `holesail.key` as a string; error records have no `holesail` - then `input.key` (a number gives its decimal form).
    pub fn hs_key(&self) -> String {
        let key = match &self.holesail {
            Some(hs) => hs.key.as_ref(),
            None => self.input.key.value.as_ref(),
        };
        match key {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Number(number)) => number.to_string(),
            _ => String::new(),
        }
    }

    /// JS truthiness of `holesail.secure`: `null` is no, the string `'false'` is yes.
    pub fn secure(&self) -> bool {
        let secure = match &self.holesail {
            Some(hs) => hs.secure.as_ref(),
            None => self.input.secure.value.as_ref(),
        };
        match secure {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(text)) => !text.is_empty(),
            _ => false,
        }
    }

    /// `holesail.secure === true` (this is how `HolesailServer` chooses `z32(seed)` or `z32(public_key)`).
    pub fn strict_secure(&self) -> bool {
        matches!(
            self.holesail.as_ref().and_then(|h| h.secure.as_ref()),
            Some(Value::Bool(true))
        )
    }

    pub fn res(&self, name: &str) -> &Value {
        &self.result.as_ref().expect("the record has no result")[name]
    }

    pub fn res_str(&self, name: &str) -> &str {
        self.res(name)
            .as_str()
            .unwrap_or_else(|| panic!("result.{name} is not a string"))
    }

    pub fn res_bytes(&self, name: &str) -> Vec<u8> {
        unhex(self.res_str(name))
    }

    pub fn error_message(&self) -> Option<&str> {
        self.error.as_ref().and_then(|e| e["message"].as_str())
    }
}

/// All records with the label "record N: role, key, secure" for assert messages.
pub fn records(role: &str) -> Vec<(String, Rec)> {
    let file: File = load("holesail/key-derivation.json");
    let all = file.vectors.into_iter().enumerate();
    let of_role = all.filter(|(_, rec)| rec.input.role == role);
    of_role.map(|(i, rec)| (label(i, &rec), rec)).collect()
}

fn label(i: usize, rec: &Rec) -> String {
    let (key, secure) = (&rec.input.key.value, &rec.input.secure.value);
    format!(
        "record {i} of key-derivation.json: {} key={key:?} secure={secure:?}",
        rec.input.role
    )
}

/// Rows of a section (`private`, `publicNoKey`) of the file `holesail-cli/keyderivation-vectors.json`.
pub fn cli_rows(section: &str) -> Vec<Value> {
    let file = load_value("holesail-cli/keyderivation-vectors.json");
    let rows = file[section].as_array();
    rows.unwrap_or_else(|| panic!("no section {section}"))
        .clone()
}

/// String field of a vector row.
pub fn field<'a>(row: &'a Value, name: &str) -> &'a str {
    row[name]
        .as_str()
        .unwrap_or_else(|| panic!("field {name} is not a string: {row}"))
}

pub fn unhex(text: &str) -> Vec<u8> {
    hex::decode(text).unwrap_or_else(|e| panic!("not hex {text:?}: {e}"))
}

pub fn unhex32(text: &str) -> [u8; 32] {
    unhex(text)
        .try_into()
        .unwrap_or_else(|_| panic!("not 32 bytes: {text:?}"))
}

/// Randomness source `00 01 … 1f`; counts calls and requires exactly 32 bytes per request.
pub fn counting_random(calls: &Cell<usize>) -> impl FnMut(&mut [u8]) + '_ {
    move |buf: &mut [u8]| {
        calls.set(calls.get() + 1);
        assert_eq!(buf.len(), 32, "randomness requested for exactly 32 bytes");
        for (byte, value) in buf.iter_mut().zip(0u8..) {
            *byte = value;
        }
    }
}
