# Legacy workbook fixture

`legacy-93f3407.rcal` was generated with IronCalc commit
`93f3407f822df5925d8b65a23248fd40155579e9` and bitcode 0.6.9.
It contains the RCAL version 1 header and `UserModel::to_bytes()` output.

The workbook is named `Legacy workbook`. Its locale and language are `en`.
Its timezone is `UTC`.

| Cell | Input | Saved result |
| --- | --- | --- |
| A1 | `42` | `42` |
| B1 | `=A1*2` | `84` |
| A2 | `=1/0` | `#DIV/0!` |
| A3 | `legacy text` | `legacy text` |

The generator used `UserModel::new_empty`, `set_user_input`, and `evaluate`.
It prepended `b"RCAL\x01"` to the serialized model.
Do not regenerate this fixture with the current engine.
