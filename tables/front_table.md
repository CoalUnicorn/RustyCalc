Based on the codebase context provided, I can see information about how tables are structured and referenced in IronCalc, but the actual API for creating tables from selected ranges is not visible in the current context. Let me share what I can find about tables.

## Table Structure and References

Tables in IronCalc are stored as part of the `Workbook` structure in a `HashMap<String, Table>` where the key is the table name [1](#0-0) .

### Table Reference Parsing

The parser handles structured references to tables with the following syntax patterns:

- `TableName[ColumnName]` - References a specific column
- `TableName[[#This Row], [ColumnName]]` - References current row in a column
- `TableName[[#Totals]]` - References the totals row
- `TableName[[#Headers]]` - References the header row
- `TableName[[#Data]]` - References the data body
- `TableName[[#All]]` - References the entire table including headers and totals [2](#0-1) 

### Table Specifiers

The system supports these table specifiers:
- `ThisRow` - Current row context
- `Totals` - Totals row (returns #REF! if no totals exist)
- `Headers` - Header row only
- `Data` - Data body (excludes headers)
- `All` - Entire table including headers and totals [3](#0-2) 

### Table Properties

From the parsing logic, tables have these properties:
- `name` - Table identifier
- `sheet_name` - Which sheet contains the table
- `reference` - Range reference (e.g., "A1:C10")
- `header_row_count` - Number of header rows
- `totals_row_count` - Number of totals rows [4](#0-3) 

## Missing Creation API

Unfortunately, the actual API for creating tables from selected ranges is not visible in the provided code context. The code I can see shows:
- How tables are stored in the workbook
- How table references are parsed in formulas
- How structured references are resolved

But the creation method (likely something like `create_table_from_range()`) is not present in these snippets. The commented code in `model.rs` suggests there was work being done on table handling [5](#0-4) , but the actual creation API is not exposed.

## Notes

The provided context focuses heavily on table reference parsing and formula evaluation rather than table creation. To find the actual table creation API, you would need to look at:
- User model methods for table creation
- WASM bindings for table operations
- UI components that handle table creation

The current context shows that IronCalc has robust support for structured references to existing tables in formulas, but the creation API is not visible in these snippets.

Wiki pages you might want to explore:
- [File I/O and Data Management (ironcalc/IronCalc)](/wiki/ironcalc/IronCalc#4)
- [Glossary (ironcalc/IronCalc)](/wiki/ironcalc/IronCalc#8)

### Citations

**File:** base/src/types.rs (L52-52)
```rust
    pub tables: HashMap<String, Table>,
```

**File:** base/src/expressions/parser/mod.rs (L919-1091)
```rust
            TokenType::StructuredReference {
                table_name,
                specifier,
                table_reference,
            } => {
                // We will try to convert to a normal reference
                // table_name[column_name] => cell1:cell2
                // table_name[[#This Row], [column_name]:[column_name]] => cell1:cell2
                let context = &self.context;
                let context_sheet_index = match self.get_sheet_index_by_name(&context.sheet) {
                    Some(i) => i,
                    None => {
                        return Node::ParseErrorKind {
                            formula: self.lexer.get_formula(),
                            position: 0,
                            message: format!("sheet not found: {}", context.sheet),
                        };
                    }
                };
                // table-name => table
                let table = match self.tables.get(&table_name) {
                    Some(t) => t,
                    None => {
                        let message = format!(
                            "Table not found: '{table_name}' at '{}!{}{}'",
                            context.sheet,
                            number_to_column(context.column)
                                .unwrap_or(format!("{}", context.column)),
                            context.row
                        );
                        return Node::ParseErrorKind {
                            formula: self.lexer.get_formula(),
                            position: 0,
                            message,
                        };
                    }
                };
                let table_sheet_index = match self.get_sheet_index_by_name(&table.sheet_name) {
                    Some(i) => i,
                    None => {
                        return Node::ParseErrorKind {
                            formula: self.lexer.get_formula(),
                            position: 0,
                            message: format!("table sheet not found: {}", table.sheet_name),
                        };
                    }
                };

                let sheet_name = if table_sheet_index == context_sheet_index {
                    None
                } else {
                    Some(table.sheet_name.clone())
                };

                // context must be with tables.reference
                #[allow(clippy::expect_used)]
                let (column_start, mut row_start, column_end, mut row_end) =
                    parse_range(&table.reference).expect("Failed parsing range");

                let totals_row_count = table.totals_row_count as i32;
                let header_row_count = table.header_row_count as i32;
                row_end -= totals_row_count;

                match specifier {
                    Some(token::TableSpecifier::ThisRow) => {
                        row_start = context.row;
                        row_end = context.row;
                    }
                    Some(token::TableSpecifier::Totals) => {
                        if totals_row_count != 0 {
                            row_start = row_end + 1;
                            row_end = row_start;
                        } else {
                            // Table1[#Totals] is #REF! if Table1 does not have totals
                            return Node::ErrorKind(token::Error::REF);
                        }
                    }
                    Some(token::TableSpecifier::Headers) => {
                        row_end = row_start;
                    }
                    Some(token::TableSpecifier::Data) => {
                        row_start += header_row_count;
                    }
                    Some(token::TableSpecifier::All) => {
                        if totals_row_count != 0 {
                            row_end += 1;
                        }
                    }
                    None => {
                        // skip the headers
                        row_start += header_row_count;
                    }
                }
                match table_reference {
                    None => Node::RangeKind {
                        sheet_name,
                        sheet_index: table_sheet_index,
                        absolute_row1: true,
                        absolute_column1: true,
                        row1: row_start,
                        column1: column_start,
                        absolute_row2: true,
                        absolute_column2: true,
                        row2: row_end,
                        column2: column_end,
                    },
                    Some(TableReference::ColumnReference(s)) => {
                        let column_index = match get_table_column_by_name(&s, table) {
                            Some(s) => s + column_start,
                            None => {
                                return Node::ParseErrorKind {
                                    formula: self.lexer.get_formula(),
                                    position: self.lexer.get_position() as usize,
                                    message: format!("Expecting column: {s} in table {table_name}"),
                                };
                            }
                        };
                        if row_start == row_end {
                            return Node::ReferenceKind {
                                sheet_name,
                                sheet_index: table_sheet_index,
                                absolute_row: true,
                                absolute_column: true,
                                row: row_start,
                                column: column_index,
                            };
                        }
                        Node::RangeKind {
                            sheet_name,
                            sheet_index: table_sheet_index,
                            absolute_row1: true,
                            absolute_column1: true,
                            row1: row_start,
                            column1: column_index,
                            absolute_row2: true,
                            absolute_column2: true,
                            row2: row_end,
                            column2: column_index,
                        }
                    }
                    Some(TableReference::RangeReference((left, right))) => {
                        let left_column_index = match get_table_column_by_name(&left, table) {
                            Some(f) => f + column_start,
                            None => {
                                return Node::ParseErrorKind {
                                    formula: self.lexer.get_formula(),
                                    position: self.lexer.get_position() as usize,
                                    message: format!(
                                        "Expecting column: {left} in table {table_name}"
                                    ),
                                };
                            }
                        };

                        let right_column_index = match get_table_column_by_name(&right, table) {
                            Some(f) => f + column_start,
                            None => {
                                return Node::ParseErrorKind {
                                    formula: self.lexer.get_formula(),
                                    position: self.lexer.get_position() as usize,
                                    message: format!(
                                        "Expecting column: {right} in table {table_name}"
                                    ),
                                };
                            }
                        };
                        Node::RangeKind {
                            sheet_name,
                            sheet_index: table_sheet_index,
                            absolute_row1: true,
                            absolute_column1: true,
                            row1: row_start,
                            column1: left_column_index,
```

**File:** base/src/model.rs (L1300-1308)
```rust
        // add all tables
        // let mut tables = Vec::new();
        // for worksheet in worksheets {
        //     let mut tables_in_sheet = HashMap::new();
        //     for table in &worksheet.tables {
        //         tables_in_sheet.insert(table.name.clone(), table.clone());
        //     }
        //     tables.push(tables_in_sheet);
        // }
```
