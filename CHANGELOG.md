# Changelog

What changed for users in each Ketchup release. The full notes, downloads, and checksums are on the
[GitHub releases page](https://github.com/pietrobb/Ketchup/releases).

## 0.3.2 — 2026-11-04

- **Connect AI** now lists Claude Code, Codex, Claude Desktop and Supervisor, and its **Connect** button writes Ketchup into the client's own settings file. It works even when the client's command is not found by the Ketchup window, where before only *Copy* was offered. Connecting again replaces an old path, such as a Supervisor copy of Ketchup. Each Supervisor instance reads its own `skills\mcp_servers.json`, so every running Supervisor with the MCP bridge (a project's instance, a development checkout), besides the installed one, gets its own row showing that file, and Connect writes only that file.
- **Connect AI** shows three numbered steps, each ticked once the window sees it done. The status line names the connected AI, for example *Connected AI: Claude Code*, next to **Disconnect**. This replaces the small window that called it an "unverified local application".
- With several Ketchup windows open, an AI connects to the one you last clicked **Connect** in, instead of asking which of several *Untitled* windows to use. The AI tab shows the short name the AI knows the window by, and when it still has to ask, the AI lists the windows by that name.
- While an AI tool call waits for the Ketchup window, the AI client still gets answers to its pings, and a call it cancels stops at once instead of running on for minutes.
- When a second AI connects to a window another AI is using, the first one is told whether the document changed while its last request ran, instead of "may or may not have been applied". The list of windows also says that connecting takes such a window over, so an AI no longer waits for the other one to leave.
- An AI can share a saved view with the Viewer: `save_view` takes `viewer` (true shares it, false leaves it out) and `saved_views` says which views are Viewer scenes; unreadable Viewer settings refuse `viewer` as `viewer_settings_unreadable` and save nothing. A Viewer export through MCP used to stop at "No shared saved scene… tick Viewer", which only the window could tick.
- The sections and elevations of the project drawings carry level marks for the building office: ±0,000 at the ground floor (the lowest floor at or above zero, so a basement is below it), or at a height the sheet settings give, then every floor, gallery and landing a person stands on, the top, and the bottom of the model below ±0,000, in metres. Hiding the floor slab no longer moves ±0,000.
- The vertical dimensions a program draws with `dim()`, which a plan cannot show, appear in the sections and elevations that show their true length (a section only those beyond its cut); the sheet summary counts the dimensions of all its views.
- A lofted part can be hollowed with `shell()`, open at its first or last section (`open=["start"]`, `["end"]` or both), e.g. a membrane over a frame; it used to be refused, so the inner loft had to be subtracted by hand.
- 0.3.1 reads some drawings differently from 0.3.0 (skipped or empty entities, CP1250 text, files with a BOM), so it often refused to export a drawing edited in 0.3.0. Most such drawings export again, also one that declares no unit: the edits find their entities by DXF handle, and an entity not edited is written as the file has it. A moved block insert that this version reads differently from 0.3.0 is still refused.
- The floor plans of the project drawings show the visible layers of an imported DXF or DWG underlay, thin and grey beneath the model. A large map such as a cadastral one is cut at the edge of the plan, so it does not make the sheet larger.
- A program can group parts that do not touch, such as the kerbs, doors and technology of a site, with `group(..., kind="logical")`. Such a group is only for finding, showing and hiding them together, so the contact check no longer reports it as falling apart; an assembly inside it is still checked. A document with a logical group is saved in the new format 108, which Kečup 0.2.0 to 0.3.1 refuse to open as newer instead of losing the mark (do not open it in 0.1.0, see the README); any other document is saved as before.
- The preview above the value field writes its numbers the way you type them: with a decimal comma where the system uses one (`= 40,6 mm`, `= 12,5; 3; 0 mm`, `= 80,6 mm (+Z)`), for lengths and expressions, radii, a rectangle's sides and every Move value alike; it used to write a point.
- The Windows installer and the portable package carry the Visual C++ runtime, so Ketchup starts on a Windows without the Visual C++ Redistributable, which 0.3.1 needed.
- The right dock shows Layers, Program, Checks, Views and AI as tabs side by side. The Program tab colours and wraps the code and marks the lines of the selected part, and its part list gives each part's real size instead of 0 × 0 × 0.
- **File > Open Recent** lists the documents opened last, and the import and export formats sit in submenus.
- A typed Move goes the way the pointer or the locked axis points, and its preview shows the distance with its direction (`= 80,6 mm (+Z)`). Typed without any direction, it moves nothing.
- On Linux and macOS, opening a document right after saving it is no longer refused as changed by another program.
- A drawings export sent over MCP while the exact evaluation still runs waits for it, up to 5 minutes, instead of being refused. A plan leaves out an upright part such as a standing pipe and names it, instead of refusing the whole plan.
- A timber member counts as braced against lateral buckling only where its bracing leads to something that holds it: a grounded part or a support. A sheet rigid in its plane (a diaphragm) cannot be declared yet, so sheathing alone does not brace and such members stay `not_verified`. A board on two free beams, laths lying loose along a beam or fixed only in line with it, and a logical group marked grounded no longer brace it, and screws brace only at their positions.
- The cutting plan takes the length axis of a profile from the attribute reserved for it, so a sloped rafter with an attribute such as `grid='X'` gets its long, narrow blank again instead of a near-square one.
- **Connect** edits Codex's `config.toml` as TOML, so a commented header, an inline table, a dotted key or a multi-line string no longer leave a file Codex refuses; a file it cannot read is left alone. A client's settings keep their permissions, symlink and other entries, with a backup of the first copy; only the ketchup server's command, args and type change, a project's own entry is seen, and the installer no longer replaces a working registration.
- **Connect AI** reads a Supervisor's `$VAR` and `${VAR}` as Supervisor does, and says why a settings file or the list of running instances could not be read.
- A drawing export refuses to take out an entity that now lies elsewhere in the file, and says how many entities changed in the file, or were added to it, since the import.
- A DXF older than 2007 is read in the right code page more often: a sign such as ², Ø or ¹ no longer turns a western drawing to Central European, real UTF-8 is recognised by its longer characters and by most of its text, and a byte order mark no longer spoils it. Text written into such a drawing uses its code page, with `\U+XXXX` only for what the page lacks.
- Data taken from a drawing into a program cannot be named like a builtin (`range`, `len`, a library function) or like a name the program already sets, such as another data's `A_POINTS`.
- An AI's export replaces a PDF or a drawing sheet only when it is the file the window last wrote there, or when the AI asks with `replace: true`; a PDF you changed or someone else's DWG is kept and named.
- When a second AI takes the window over, or you press **Disconnect**, an export that was running puts back the sheet settings it changed and stops, and the AI is told the document changed if they did. AI tool calls run one at a time in the order they came; a call cancelled while it waits never runs, nor do calls still waiting when the AI leaves.
- The static FEA review runs beside the window, which keeps painting and shows it running, and **Cancel** stops it without changing the document.
- **File > New** keeps the decimal separator, the remembered language and the dock tab, so `12,5` is still read on Czech or Slovak Windows after New.
- **Open Recent** checks whether its documents exist once when it opens, waiting at most 100 ms, so an entry on a server that does not answer no longer freezes the window; such an entry stays listed and is named as out of reach when clicked. On macOS and Linux the list and the chosen language are kept after a restart.
- The Program tab stays fast with a program of thousands of lines.
- An axis locked in the middle of a Move drag keeps the way the drag went, so a typed distance goes there.
- The Checks badge says `99+` above 99, and a value that rounds to zero shows `0`, not `-0`.
- Czech and Slovak drawing sets write every number with a decimal comma, in dimensions and cutting-plan captions too (`1234,5`, `využito 95,2 %`).
- A drawing set belongs to its project file: a copy saved under another name no longer replaces its original's set, an export stopped between the sheets and their list no longer blocks the next one, and a failed roll back says which sheet it could not put back and where its backup is.
- A plan's underlay is grey on a layer of its own in the DXF, and a shown underlay whose lines cannot be read stops the drawings and is named instead of being left out. A part a plan leaves out is named as the model names it.
- A worktop with the opening for its sink is made with the opening cut through it, and woodWOP refuses that cut by name. A woodWOP pocket with an invalid tool and a package with nothing to program say so.
- The Windows package's notices name the Visual C++ runtime it carries, the runtime is the one of the toolset that built Ketchup, and a signed package signs the OCCT libraries too.
- On Linux an AI cannot reach a file on a FUSE network mount such as sshfs, rclone or s3fs through a local-looking path.

Files from 0.3.1 open in 0.3.2. A file with a logical group needs 0.3.2.

## 0.3.1 — 2026-11-02

**Structural checks and production**
- A compressed timber edge counts as braced only where a board or sheathing really covers it; otherwise its lateral buckling is checked.
- Boards such as OSB/3 are checked with their own factors, not those of solid timber.
- An extrusion's blank is the smallest rectangle around its profile, so a sloped gable rafter gets a realistic blank.
- The cut list and the cutting plan give dimensions in the same order. A connector that doesn't fit its members still appears in the hardware list with its nails.
- CNC DXF draws a thread on its own layer or names the part it cannot draw. BTLx saw and mill contours carry the depth of the cut, and a blocked part export names the part.

**Files and imports**
- A STEP or IGES file with some broken solids imports the rest and names each solid left out with its source entity and the reason, in the import review, the status line, and over MCP.
- STEP and IGES imports started over MCP run in the background and can be cancelled.
- Starting Ketchup with a file that isn't a Ketchup document shows a message instead of a general error.
- A sheet whose text has a character its typeface lacks refuses only the PDF; its SVG, DXF, and DWG are still written.
- Hatches and arcs of an edited drawing keep their angle range when exported back to DXF or DWG.

**Programs and modeling**
- A whole-number parameter refuses a decimal value. Pushing a face all the way through its part is refused instead of leaving a broken part.

**Safety and licences**
- A plugin argument naming a file in the plugin's folder is never read as an option.
- Each download carries third-party notices with the copyright and licence text of every component it contains.
- An Android Viewer release no longer measures a pushed `benchmark.ketchup-view`; only a `-Benchmark` build does.

**Also fixed**
- woodWOP refuses by name a blind hole as deep as its part; BTLx and the production job drill it through.
- Drawing PDFs embed the sheet typeface, so ö, ü, ß, ř, ě, ů and ł print.
- The MCP connection survives a reset or aborted client instead of hanging the window.
- Files saved by 0.3.1 never make 0.2.0 open the older `.recovery` copy: 0.2.0 refuses a file it cannot read and leaves it unchanged.
- The portable Ketchup Viewer runs on a clean Windows without the Visual C++ Redistributable.
- A Ketchup.app built from 0.3.1 carries the program worker, so program evaluation, DWG import and underlay export work on macOS. 0.3.1 published no macOS download.

Files from 0.3.0 open in 0.3.1. A file with a trimmed, extended or offset drawing line needs 0.3.1.

**Known issue:** Ketchup 0.3.1, installed or portable, does not start on a Windows without the [Microsoft Visual C++ Redistributable](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist) for x64, version 14.29 or later. Install it, then start Ketchup again. From 0.3.2 on, the runtime comes with the programs.

## 0.3.0 — 2026-10-30

- DXF and DWG drawings under the model (underlays): snap to them, fit them by two points, edit their lines and texts, pull a closed area up into a solid, stand a drawing in space as a view, and export it back to DXF or DWG.
- Rule programs: `window()` and `door()`, a **Program parameters** section, window edits written back into the program, `from_underlay`.
- Drawing sets as DXF or DWG sheets with shop drawings of build-ups and window and door marks; cutting plans from `bar_stock()` and `sheet_stock()`; quotes from `price()`; a fastener catalogue (`connector()`); CNC DXF of flat parts.
- COLLADA (DAE) export for SketchUp; STL, 3MF, GLB, and DAE exports in the background; faster exact checks after an edit.
- A document is saved in the oldest format that holds it.
- Ketchup Viewer: language and projection settings, program dimensions, orthographic plans, and a portable download of its own.
- **Connect AI** and `Ketchup.exe --mcp-setup` set up Claude Code and Codex.
- Ketchup and Ketchup Viewer build on Apple Silicon Macs. Correction: a Ketchup.app built from 0.3.0 lacks the program worker, so program evaluation and DWG import fail there; 0.3.0 published no macOS download. Build from 0.3.1 or later.

Files from 0.2.0 open in 0.3.0. A file with underlays or annotations needs 0.3.0.

## 0.2.0 — 2026-10-18

- Every value field reads values the same way: the system decimal separator, units, and arithmetic. Scale has grips on the selection box.
- Sketch, Push/Pull, Move, Rotate, and Scale each make one Undo step; lines drawn on a body lie on its face.
- Tapped holes use shop tap drills and the DIN 76-1 runout. Mirrored parts get their own BOM row and drawing. Timber members are checked on their real section, or reported as *not verified*.
- Rule programs and DWG reading run in a separate process with memory and time limits. STEP and IGES export run in the background with Cancel.
- Plugins run from a hashed private copy; a file from a newer Ketchup is refused instead of losing data.
- Ketchup Viewer picks faster; on Android it opens packages of 120 MB and recovers an interrupted save.

Files from 0.1.0 open in 0.2.0. A file saved by 0.2.0 may need 0.2.0 to open.

## 0.1.0 — 2026-10-14

First public release: an open-source 2D/3D parametric modeler for Windows with exact geometry, rule programs, production exports, Ketchup Viewer, and AI editing over MCP that goes through the same validation and Undo as a manual edit.
