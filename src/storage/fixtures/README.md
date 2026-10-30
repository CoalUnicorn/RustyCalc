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

## RCAL v2 fixture

`legacy-7e6ae21c.rcal` was generated with IronCalc commit
`7e6ae21c0f163cacf56b9d47acf777454bfaa243` and bitcode 0.6.9.
It contains `b"RCAL\x02"` followed by `UserModel::to_bytes()` output.
Its SHA-256 is `ea036b19091accb036c5d395ba4861e5d657a6920f49dfea1ca1782c60846f49`.
Do not regenerate it with the current engine.

The generator used `UserModel::new_empty("Legacy workbook", "en", "UTC", "en")`.
It set the same four cells as the v1 fixture. It also set these properties:

- `Sheet1!Z1000` contains `sparse cell`.
- `Sheet1!A1` has bold font.
- The defined name `answer` refers to `Sheet1!$A$1` at workbook scope.
- `Sheet1!A3` links to `https://example.com` with tooltip `Legacy link`.
- Sheet1 has one frozen row and hides grid lines.
- A second sheet contains `=Sheet1!A1+1` in A1, with result `43`.
- The selected sheet is the second sheet. Its selected cell is E10.
  Its top-left visible cell is C7.

The generator called `evaluate()` before serialization. The non-default view
fields verify that the old view data is consumed during migration. The current
engine opens the workbook on the first sheet with A1 selected.

Run the IndexedDB browser test with this fixture after a Trunk build:

```sh
RCAL_FIXTURE=legacy-7e6ae21c.rcal node tests/browser/indexeddb.mjs
```
