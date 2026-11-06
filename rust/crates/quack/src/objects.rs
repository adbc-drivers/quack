// Copyright (c) 2026 ADBC Drivers Contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//         http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::{connection::Session, error::ErrorHelper, statement::QuackStatement};
use adbc_core::Statement;
use arrow_array::{Array, Int32Array, StringArray};
use driverbase::{error::ErrorHelper as _, get_objects::*};
use std::sync::Arc;
type Result<T> = std::result::Result<T, driverbase::error::Error<ErrorHelper>>;
pub(crate) struct Objects(pub Arc<Session>, pub Option<String>);
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
fn pattern(s: Option<&str>) -> String {
    literal(s.unwrap_or("%"))
}
impl Objects {
    fn rows(&self, sql: String) -> Result<Vec<Vec<Option<String>>>> {
        let mut statement = QuackStatement::new(self.0.clone());
        statement
            .set_sql_query(sql)
            .map_err(|e| ErrorHelper::status(e.status).message(e.message))?;
        let reader = statement
            .execute()
            .map_err(|e| ErrorHelper::status(e.status).message(e.message))?;
        let mut rows = Vec::new();
        for batch in reader {
            let batch = batch.map_err(|e| {
                ErrorHelper::invalid_data()
                    .context("read metadata")
                    .message(e.to_string())
            })?;
            for i in 0..batch.num_rows() {
                let mut row = Vec::new();
                for a in batch.columns() {
                    row.push(if a.is_null(i) {
                        None
                    } else if let Some(a) = a.as_any().downcast_ref::<StringArray>() {
                        Some(a.value(i).to_owned())
                    } else if let Some(a) = a.as_any().downcast_ref::<Int32Array>() {
                        Some(a.value(i).to_string())
                    } else {
                        return Err(ErrorHelper::invalid_data().message("unexpected metadata type"));
                    });
                }
                rows.push(row);
            }
        }
        Ok(rows)
    }
}
fn value(row: &[Option<String>], i: usize) -> Result<String> {
    row.get(i)
        .and_then(Clone::clone)
        .ok_or_else(|| ErrorHelper::invalid_data().message("missing metadata value"))
}
impl GetObjectsImpl<ErrorHelper> for Objects {
    fn get_catalogs(&self, filter: Option<&str>) -> Result<Vec<String>> {
        self.rows(format!("SELECT DISTINCT catalog_name FROM information_schema.schemata WHERE catalog_name LIKE {} ORDER BY 1", pattern(filter)))?.iter().map(|r| value(r,0)).collect()
    }
    fn get_db_schemas(&self, catalog: &str, filter: Option<&str>) -> Result<Vec<String>> {
        self.rows(format!("SELECT schema_name FROM information_schema.schemata WHERE catalog_name = {} AND schema_name LIKE {} ORDER BY 1",literal(catalog),pattern(filter)))?.iter().map(|r| value(r,0)).collect()
    }
    fn get_tables(
        &self,
        catalog: &str,
        schema: &str,
        filter: Option<&str>,
        types: Option<&[String]>,
    ) -> Result<Vec<TableInfo>> {
        let mut condition = String::new();
        if let Some(types) = types.filter(|t| !t.is_empty()) {
            for t in types {
                match t.as_str() {
                    "LOCAL TABLE" | "BASE TABLE" | "VIEW" => (),
                    _ => {
                        return Err(
                            ErrorHelper::invalid_argument().message("unsupported table type")
                        );
                    }
                }
            }
            condition = format!(
                " AND table_type IN ({})",
                types
                    .iter()
                    .map(|t| literal(t))
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        self.rows(format!("SELECT table_name, table_type FROM information_schema.tables WHERE table_catalog = {} AND table_schema = {} AND table_name LIKE {}{} ORDER BY 1",literal(catalog),literal(schema),pattern(filter),condition))?.iter().map(|r| Ok(TableInfo { table_name:value(r,0)?, table_type:value(r,1)? })).collect()
    }
    fn get_columns(
        &self,
        catalog: &str,
        schema: &str,
        filter: Option<&str>,
        types: Option<&[String]>,
        column: Option<&str>,
    ) -> Result<Vec<TableAndColumnInfo>> {
        self.get_tables(catalog,schema,filter,types)?.into_iter().map(|table| {
   let rows=self.rows(format!("SELECT column_name, ordinal_position::INTEGER FROM information_schema.columns WHERE table_catalog = {} AND table_schema = {} AND table_name = {} AND column_name LIKE {} ORDER BY ordinal_position",literal(catalog),literal(schema),literal(&table.table_name),pattern(column)))?;
   let columns=rows.iter().map(|r| Ok(ColumnInfo {column_name:value(r,0)?, ordinal_position:Some(value(r,1)?.parse().map_err(|e: std::num::ParseIntError| ErrorHelper::invalid_data().message(e.to_string()))?), remarks:Some(String::new()), ..Default::default()})).collect::<Result<Vec<_>>>()?;
   Ok(TableAndColumnInfo {table,columns})
  }).collect()
    }
    fn get_constraints(
        &self,
        catalog: &str,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ConstraintInfo>> {
        let rows = self.rows(format!(
            "SELECT constraint_index::VARCHAR, constraint_name, constraint_type,              unnest(constraint_column_names), referenced_table, unnest(referenced_column_names)              FROM duckdb_constraints() WHERE database_name = {} AND schema_name = {}              AND table_name = {} AND constraint_type <> 'NOT NULL' AND list_has_any(constraint_column_names, list_filter(constraint_column_names, lambda name: name LIKE {})) ORDER BY constraint_index",
            literal(catalog), literal(schema), literal(table), pattern(self.1.as_deref())
        ))?;
        let mut constraints: Vec<ConstraintInfo> = Vec::new();
        let mut previous = None;
        for row in rows {
            let identity = value(&row, 0)?;
            if previous.as_ref() != Some(&identity) {
                constraints.push(ConstraintInfo {
                    constraint_name: Some(value(&row, 1)?),
                    constraint_type: value(&row, 2)?,
                    constraint_column_names: Vec::new(),
                    constraint_column_usage: if value(&row, 2)? == "FOREIGN KEY" {
                        Some(Vec::new())
                    } else {
                        None
                    },
                });
                previous = Some(identity);
            }
            if let Some(constraint) = constraints.last_mut() {
                constraint.constraint_column_names.push(value(&row, 3)?);
                if let Some(usage) = constraint.constraint_column_usage.as_mut() {
                    usage.push(ConstraintColumnUsage {
                        fk_catalog: Some(catalog.to_owned()),
                        fk_db_schema: Some(schema.to_owned()),
                        fk_table: value(&row, 4)?,
                        fk_column_name: value(&row, 5)?,
                    });
                }
            }
        }
        Ok(constraints)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adbc_core::options::{OptionDatabase, OptionValue};
    use adbc_core::{Database, Optionable};

    #[test]
    fn escaped_literals() {
        assert_eq!(literal("a'b"), "'a''b'");
        assert_eq!(pattern(None), "'%'");
        assert_eq!(pattern(Some("")), "''");
    }

    #[test]
    fn live_metadata() {
        let endpoint = match std::env::var("QUACK_HTTP_ENDPOINT") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => return,
            Err(error) => panic!("{error}"),
        };
        let endpoint = url::Url::parse(&endpoint).unwrap();
        assert_eq!(endpoint.scheme(), "http");
        let mut database = crate::database::QuackDatabase::new().unwrap();
        database
            .set_option(
                OptionDatabase::Uri,
                OptionValue::String(format!(
                    "quack://{}/?token=quack-secret&tls=false",
                    &endpoint[url::Position::BeforeHost..url::Position::AfterPort]
                )),
            )
            .unwrap();
        let connection = database.new_connection().unwrap();
        let objects = Objects(connection.session.clone(), None);
        let name = format!("metadata_{}", uuid::Uuid::new_v4().simple());
        let foreign = format!("{name}_fk");
        let mut statement = QuackStatement::new(connection.session.clone());
        statement
            .set_sql_query(format!(
                "CREATE TABLE {name} (a INTEGER, b INTEGER, PRIMARY KEY(a,b), CHECK(a > 0))"
            ))
            .unwrap();
        statement.execute_update().unwrap();
        statement.set_sql_query(format!("CREATE TABLE {foreign} (x INTEGER, y INTEGER, FOREIGN KEY(x,y) REFERENCES {name}(a,b))")).unwrap();
        statement.execute_update().unwrap();
        let catalog = value(
            &objects.rows("SELECT current_database()".into()).unwrap()[0],
            0,
        )
        .unwrap();
        let columns = objects
            .get_columns(&catalog, "main", Some(&name), None, Some("b"))
            .unwrap();
        assert_eq!(columns[0].columns[0].ordinal_position, Some(2));
        assert!(
            objects
                .get_tables(&catalog, "main", Some(&name), Some(&["VIEW".into()]))
                .unwrap()
                .is_empty()
        );
        let constraints = objects.get_constraints(&catalog, "main", &name).unwrap();
        assert!(
            constraints
                .iter()
                .any(|c| c.constraint_type == "PRIMARY KEY"
                    && c.constraint_column_names == ["a", "b"])
        );
        assert!(constraints.iter().any(|c| c.constraint_type == "CHECK"));
        let constraints = objects.get_constraints(&catalog, "main", &foreign).unwrap();
        let fk = constraints
            .iter()
            .find(|c| c.constraint_type == "FOREIGN KEY")
            .unwrap();
        assert_eq!(fk.constraint_column_names, ["x", "y"]);
        let usage = fk.constraint_column_usage.as_ref().unwrap();
        assert_eq!(
            usage
                .iter()
                .map(|u| u.fk_column_name.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        for table in [&foreign, &name] {
            statement
                .set_sql_query(format!("DROP TABLE {table}"))
                .unwrap();
            statement.execute_update().unwrap();
        }
    }
}
