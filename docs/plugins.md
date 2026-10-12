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
  the process runs in the folder with `arguments` and an empty environment.
- MCP `run_plugin` takes `plugin` (the folder) and `inputs`; it cannot change
  or extend the command line or the permissions.
- Inputs are `number`, `integer`, `boolean` or `string`. The plugin reads them
  with `Client.inputs()` (`QUERY\tINPUTS`), never from its arguments. An
  undeclared, missing or mistyped input is refused before anything runs.

- A link (symlink or junction) in the folder, or as the folder, is refused.

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
input values do not. A Python plugin runs with `PYTHONDONTWRITEBYTECODE=1`, so
it does not change its own folder.

Known limit: a file can still be replaced in the few milliseconds between
hashing it and starting the process.
