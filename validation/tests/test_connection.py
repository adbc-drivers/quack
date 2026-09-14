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

import os
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

import adbc_driver_manager.dbapi
import adbc_drivers_validation.tests.connection as connection_tests
import pytest

from .quack import get_quirks


def pytest_generate_tests(metafunc) -> None:
    quirks = [get_quirks(metafunc.config.getoption("vendor_version"))]
    return connection_tests.generate_tests(quirks, metafunc)


def with_tls(uri: str, tls: str) -> str:
    parts = urlsplit(uri)
    query = [
        (key, value)
        for key, value in parse_qsl(parts.query, keep_blank_values=True)
        if key != "tls"
    ]
    query.append(("tls", tls))
    return urlunsplit(parts._replace(query=urlencode(query)))


class TestConnection(connection_tests.TestConnection):
    def test_http_query(self, driver_path: str, db_kwargs: dict) -> None:
        uri = os.environ.get("QUACK_HTTP_URI")
        if not uri:
            pytest.skip("QUACK_HTTP_URI is required for the plaintext endpoint")
        options = {**db_kwargs, "uri": uri}
        with adbc_driver_manager.dbapi.connect(
            driver=driver_path, db_kwargs=options, autocommit=True
        ) as connection:
            with connection.cursor() as cursor:
                cursor.execute("SELECT 1")
                assert cursor.fetchone() == (1,)

    @pytest.mark.parametrize("tls_source", ["uri", "database"])
    def test_tls_rejects_untrusted_certificate(
        self, driver_path: str, db_kwargs: dict, tls_source: str
    ) -> None:
        options = {**db_kwargs, "uri": with_tls(db_kwargs["uri"], "skip_verify")}
        options.pop("quack.tls", None)
        with adbc_driver_manager.dbapi.connect(
            driver=driver_path, db_kwargs=options, autocommit=True
        ) as connection:
            with connection.cursor() as cursor:
                cursor.execute("SELECT 1")
                assert cursor.fetchone() == (1,)

        if tls_source == "uri":
            options["uri"] = with_tls(options["uri"], "true")
        else:
            options["quack.tls"] = "true"
        with pytest.raises(adbc_driver_manager.dbapi.OperationalError) as excinfo:
            with adbc_driver_manager.dbapi.connect(
                driver=driver_path, db_kwargs=options, autocommit=True
            ):
                pass
        assert excinfo.value.status_code == adbc_driver_manager.AdbcStatusCode.IO
