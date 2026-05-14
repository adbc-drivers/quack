-- Copyright (c) 2026 ADBC Drivers Contributors
--
-- Licensed under the Apache License, Version 2.0 (the "License");
-- you may not use this file except in compliance with the License.
-- You may obtain a copy of the License at
--
--         http://www.apache.org/licenses/LICENSE-2.0
--
-- Unless required by applicable law or agreed to in writing, software
-- distributed under the License is distributed on an "AS IS" BASIS,
-- WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
-- See the License for the specific language governing permissions and
-- limitations under the License.

FORCE INSTALL quack FROM core_nightly;
LOAD quack;
CALL quack_serve(
    'quack:0.0.0.0:9494',
    token => 'quack-secret',
    disable_ssl => true,
    allow_other_hostname => true
);
CREATE TABLE IF NOT EXISTS quack_validation_ready AS SELECT 1 AS ok;
