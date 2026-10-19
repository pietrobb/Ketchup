# Plugins

A plugin is a local program that reads the document and sends one typed CAD
program, written as for MCP `model apply_and_verify`. The window publishes it
as one Undo step, only while the document is still at the revision the plugin
read. Protocol: `ketchup.plugin.v1`; Python SDK: `sdk/python/ketchup_sdk`.

## Plugin folder

A plugin folder holds `plugin.json` and everything the plugin runs, including
modules it imports (for Python, vendor `ketchup_sdk` into the folder):

```json
{
  "name": "Height",
  "executable": "C:\\Python312\\python.exe",
  "arguments": ["plugin.py"],
  "principal_id": 7001,
  "capabilities": ["query.agent-state.v1", "edit.cad-program.v1"],
  "allowed_operations": ["set_dimension"],
  "inputs": {
    "height_mm": {"type": "number"},
    "feature_id": {"type": "integer", "default": 2}
  }
}
```

- `executable` is an absolute path (an interpreter) or a file inside the folder;
  the process runs in a copy of the folder (see Consent) with `arguments` and
  an empty environment. A relative argument naming a file outside the folder
  is passed as its absolute path. The command line names a script (or the
  executable is in the folder), so everything that runs is hashed. An
  argument is a file or plain words (letters, digits and `-_.,+/\`, no part
  that starts at a root or ends in `..`), or an option `-x`, `-xVALUE`,
  `--name`, `--name=VALUE` or `name=VALUE` with a name of letters, digits,
  `-` and `_` and a value of plain words, checked again after every `=`.
  Refused are an argument starting with `@` (arguments read from a file), a
  URL, network share, device or stream, and the options that run code given
  as text, from standard input or found by name: `-`, the short options `-c`,
  `-e`, `-E`, `-m`, `-M`, `-p`, `-r` and `-i` (alone, with a joined value or
  anywhere in a cluster of letters such as `-Bm`) and the long options
  `--eval`, `--print`, `--require`, `--import`, `--loader`,
  `--experimental-loader`, `--module`, `--command`, `--encodedcommand`,
  `--interactive` and `--include`, in any letter case.
- Inputs are `number`, `integer`, `boolean` or `string`. The plugin reads them
  with `Client.inputs()` (`QUERY\tINPUTS`), never from its arguments. An
  undeclared, missing or mistyped input is refused before the user is asked,
  and so are inputs that together take more than one protocol line carries
  (about 128 KiB as compact JSON); the refusal names the largest input.

- A link (symlink or junction) in the folder, or as the folder, is refused.

## MCP `run_plugin`

`run_plugin` runs either a plugin folder or a command line, never both:

- `plugin` is the absolute path of a plugin folder (or of its `plugin.json`)
  and `inputs` the values of its declared inputs. `plugin.json` alone decides
  the command line, principal and permissions: `arguments`, `principal_id`,
  `capabilities` or `allowed_operations` beside `plugin` are refused.
- `executable` without `plugin` is a command line: the local absolute path of
  the executable (on Windows the `.exe` file that starts), `arguments`,
  `principal_id` (the principal the plugin declares), `capabilities` and
  `allowed_operations`. The arguments follow the rules of `plugin.json` above:
  at least one names a script file, every file they name is hashed for the
  consent, and a word naming no file must be plain words. Give the script as
  an absolute path; a relative one is resolved against the window's working
  folder. The process runs in an empty private folder with an empty
  environment. `inputs` need a plugin folder and are refused here.
- `timeout_ms` (default 30000, at most 120000) bounds the plugin process;
  `validators`, `strict` and `expected` work as for `model apply_and_verify`.

## Plugins menu

The window lists every folder under `%APPDATA%\Ketchup\plugins` when the menu
first opens and on **Refresh list**; it does not watch the folder. **Open
plugins folder** creates it if it is missing. A folder that could not run is
listed as unavailable with the code and reason a run would get. Choosing a
plugin with inputs opens a form built from the declared inputs and defaults;
the values are checked as for MCP. A run from the menu is the MCP `run_plugin`
run: the same consent, one Undo step, and the window is busy for MCP while it
runs.

## Consent

Before a plugin first runs, the window asks the user, showing the folder with
one SHA-256 over all its files, every file outside it the command line names
(e.g. the interpreter) with its SHA-256, the command line, principal,
capabilities and operations. The consent lasts while the window is open and
holds for runs from the menu and from MCP alike. Any
changed file, another command line or a further permission asks again; other
input values do not.

What runs is what was allowed. Each run first copies the plugin folder into a
private folder of the user (`%LOCALAPPDATA%\Ketchup\plugin-stages`, not the
shared temporary folder), hashes that copy for the consent and starts the
process there, in the copy; the copy is removed when the plugin has ended. A
file changed or added in the plugin folder while the consent prompt is open, or
during the run, is not run (the next run asks again). Files written by the
plugin into its working folder are removed with the copy; a Python plugin runs
with `PYTHONDONTWRITEBYTECODE=1`. Files outside the folder (the interpreter, a
script named on the command line) are hashed through a handle kept open until
the plugin has ended; on Windows nobody can write, rename or delete them
meanwhile. On other systems only the folder copy protects the run.
