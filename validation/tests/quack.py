# Copyright (c) 2026 ADBC Drivers Contributors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#         http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import functools
from pathlib import Path

from adbc_drivers_validation import model, quirks


class QuackQuirks(model.DriverQuirks):
    name = "quack"
    driver = "adbc_driver_quack"
    driver_name = "ADBC Driver for DuckDB Quack"
    vendor_name = "DuckDB Quack"
    vendor_version = "1.5.3"
    short_version = "1.5"
    features = model.DriverFeatures(
        connection_get_table_schema=False,
        connection_set_current_catalog=True,
        connection_set_current_schema=True,
        connection_transactions=True,
        current_catalog="quack-validation",
        current_schema="main",
        get_objects=True,
        statement_bind=False,
        statement_bulk_ingest=True,
        statement_execute_schema=False,
        statement_get_parameter_schema=False,
        statement_prepare=True,
        statement_rows_affected=False,
        statement_rows_affected_ddl=False,
        secondary_catalog="quack-validation",
        secondary_schema="quack_validation_secondary",
        supported_xdbc_fields=[],
    )
    setup = model.DriverSetup(
        database={
            "uri": model.FromEnv("QUACK_URI"),
        },
        connection={},
        statement={},
    )

    @property
    def queries_paths(self) -> tuple[Path]:
        return (Path(__file__).parent.parent / "queries",)

    def is_table_not_found(self, table_name: str | None, error: Exception) -> bool:
        error_text = str(error).lower()
        return (
            "does not exist" in error_text
            or "not found" in error_text
            or "no such table" in error_text
        )

    def split_statement(self, statement: str) -> list[str]:
        return quirks.split_statement(statement, dialect="duckdb")


@functools.cache
def get_quirks(version: str, *, vendor: str = "DuckDB Quack") -> QuackQuirks:
    quack_quirks = QuackQuirks()
    if version not in {quack_quirks.vendor_version, quack_quirks.short_version}:
        raise ValueError(f"Unsupported DuckDB Quack version: {version}")
    if vendor not in {quack_quirks.name, quack_quirks.vendor_name}:
        raise ValueError(f"Unsupported Quack vendor: {vendor}")
    return quack_quirks
