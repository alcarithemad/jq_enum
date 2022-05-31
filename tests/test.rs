use json_enum::json_enum;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Properties {
    value_a: String,
    value_b: usize,
}

json_enum! {
    TestEnum1,
    "testdata/testdata.json",
    "[ to_entries | .[].key | split(\"/\")[-1] ]",
    {
        tags: Vec<String> = "[ to_entries | .[].value.tags ]",
        cls: String = "[ to_entries | .[].value.class ]",
        properties: Properties = "[ to_entries | .[].value.properties ]", // and even arbitrary types that implement serde::Deserialize
    }
}

#[test]
fn test_basic() {
    let t = TestEnum1::A1;
    assert_eq!(t.properties().value_b, 1);
    assert!(t.tags().contains(&"tag2".to_owned()));
    assert_eq!(TestEnum1::A1.properties().value_b, 1);
    assert_eq!(TestEnum1::A2.properties().value_b, 2);
    assert_eq!(TestEnum1::Var3.properties().value_b, 3);
}
