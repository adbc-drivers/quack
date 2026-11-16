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

import adbc_driver_manager.dbapi
import adbc_drivers_validation.tests.statement as statement_tests
import pytest

from .quack import get_quirks


def pytest_generate_tests(metafunc) -> None:
    quirks = [get_quirks(metafunc.config.getoption("vendor_version"))]
    return statement_tests.generate_tests(quirks, metafunc)


class TestStatement(statement_tests.TestStatement):
    @pytest.mark.parametrize(
        "sql_query",
        [
            pytest.param("SELECT [1, 2, 3]", id="list"),
            pytest.param("SELECT {'a': 1}", id="struct"),
            pytest.param("SELECT MAP([1], [2])", id="map"),
            pytest.param("SELECT 1::HUGEINT", id="hugeint"),
            pytest.param("SELECT 1::UHUGEINT", id="uhugeint"),
            pytest.param(
                "SELECT '550e8400-e29b-41d4-a716-446655440000'::UUID", id="uuid"
            ),
            pytest.param("SELECT INTERVAL '1 day'", id="interval"),
            pytest.param("SELECT TIMETZ '12:00:00+00'", id="timetz"),
            pytest.param("SELECT 'a'::ENUM('a', 'b')", id="enum"),
            pytest.param("SELECT '101'::BIT", id="bit"),
        ],
    )
    def test_unsupported_result_preserves_connection(self, conn, sql_query):
        with conn.cursor() as cursor:
            with pytest.raises(adbc_driver_manager.dbapi.NotSupportedError):
                cursor.execute(sql_query)
            cursor.execute("SELECT 1")
            assert cursor.fetchall() == [(1,)]
