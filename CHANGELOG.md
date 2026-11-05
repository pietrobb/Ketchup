# Changelog

What changed for users in each Ketchup release. The full notes, downloads, and checksums are on the
[GitHub releases page](https://github.com/pietrobb/Ketchup/releases).

## Unreleased

- A timber member counts as braced against lateral buckling only by parts fastened to it and to a part declared grounded beside it. Fastener points away from where the two parts touch hold nothing and are reported (`fastener_off_contact`); a lath resting on both arms of one grounded part along the beam, joints of kind `none`, `contact` or `friction`, a `fixed` tag, and a free stud standing on the floor no longer brace it. An area load on a part standing on edge, which takes none of it, is reported (`area_load_edge_on`).
- The installer no longer replaces an AI client's working Ketchup entry whose command is not a full path: a name found on PATH (`ketchup`), a path from `~`, or a Windows path without `.exe` is looked up as the client looks it up, and one Ketchup cannot find there is kept. **Connect AI** shows an entry that is turned off (`disabled`, Codex `enabled = false`) or a network server as starting nothing, and **Connect** turns it on as a program. The client's settings file is replaced in one step on Windows too and keeps its permissions; a link to a settings file not there yet creates that file; an error in a nested setting names its keys.
- Writing an edited drawing back no longer fails as "read differently" when the file gained far-away objects since it was imported without a move.
- The project drawings window shows the ground floor it found for ±0,000, a height you set starts from that value, and any height can be set, also one in absolute heights hundreds of metres above sea level (before, at most ±200 m).
- Ketchup 0.3.1 can again read the Viewer sharing choices of a project in which a newer version left a saved view out, and it does not share that view; before, 0.3.1 could not read the choices and refused the Viewer export.
- A DXF drawing before 2007 whose texts are UTF-8 with a few Greek letters or degree signs is read as UTF-8 (`Δh`, `α = 30°`, before `Î”h`, `Î± = 30Â°`) and the code page in its header is reported as doubtful. A drawing read in its code page that could also be UTF-8 (`ε`, `Ке` read as `Îµ`, `ÐšÐµ`) is always reported as doubtful.
- A DXF drawing whose header names Windows-1252 but whose texts read as Windows-1250 (`Kuchyně`, `Předsíň`) is written back naming Windows-1250, so after a text is deleted in Kečup the rest no longer reads as `Kuchynì` in Kečup or AutoCAD.
- Deciding the code page of a DXF drawing, a word counts once however often it is written: one Czech or Slovak layer name repeated on every entity no longer outvotes the Western texts or turns the drawing on its own; it is warned of.
- A DXF drawing whose MTEXT font (`\f…|c238;`) or text style font names the East European character set counts that as a vote for Windows-1250, so a short Czech drawing headed Windows-1252 reads `Kuchyně`, not `Kuchynì`.
- Writing an edited DXF back, a text, fill or entity moved or taken out in Kečup is refused when the file changed it since the import: a text that lies elsewhere or reads otherwise, a fill that covers another area, an entity on another layer. Before, a text moved in both was written where neither put it, and a reworded one was deleted unseen.
- Refusing to write back a large edited DXF that changed since its import takes no longer than writing it (100 000 lines: 6,5 s before, under 1 s now): the entities are indexed by handle once.
- When writing an edited drawing back is refused because an edited object changed in the file since the import, the message names the object, its entity and handle, what differs (layer, lines, text or fill) and where it lay and lies now. **Tools ▸ Discard the refused drawing edit** undoes that one edit as one Undo step and keeps the others, so the drawing can be written without importing it again.
- Writing an edited drawing back no longer refuses the whole drawing because an entity you did not edit was deleted in AutoCAD since the import: it is left out, as the file has it, and the export says how many. An edited object whose entity was deleted is refused by name and handle, and **Tools ▸ Discard the refused drawing edit** drops just that edit. The count of entities you did not edit that "changed in the file" now says they read otherwise than at import, which also covers a newer Kečup reading the same file differently.
- An AI exporting a PDF or a drawing sheet again after an edit replaces the file the document wrote there also after Ketchup was restarted, and when the path is written with other letter case (`c:\X` for `C:\x`); the document keeps which files it wrote, so it is marked changed after such an export. A file someone else changed is still refused, and the AI is told to ask you before replacing it.
- A project renamed, moved or saved as a new version exports its DXF or DWG drawing set to the same folder again instead of refusing it as changed meanwhile. Exporting to the set of another project tells you which project file it came from, and you can take it over with **Take the set over and export** (`take_over` for the AI). The set's sheet list names the project file by its name only, no longer by a full path with your user name. A set written by Kečup 0.3.2 is still your project's when you open the same project file by a differently written path (other letter case, a mapped drive or its network share) or from a recovered session.
- **File > Review static FEA** no longer shows numbers for the model as it was before an edit made while the review ran or after it: such a result is dropped and you are asked to run the review again. Opening or starting another document closes the dialog.
- When an AI client closes its connection, or its replies can no longer be delivered, the connection of the edit it was running is closed, so the window stops a job still running, and none of its queued edits are made any more; an `open_window` waiting for the new window stops when cancelled. The MCP server holds at most 16 waiting tool calls at a time, so a client sending faster than the window works no longer fills the memory.
- The level marks of sections and elevations mark a gallery of planks or OSB strips and a 900 mm landing, and no longer a bed, a table or a kitchen island. ±0,000 is found among the parts the sheets draw and those hidden one by one, no longer from a terrain, a neighbour or a variant on a hidden tag; when the exact solids cannot be read the export says so instead of putting ±0,000 at 0, and the drawings window finds the ground floor without stalling, showing "…" until then and why when it cannot.
- The date in the title block of an English sheet is written 2026-11-05, no longer the Czech and Slovak way 5. 11. 2026.
- **File > Export plan DXF** writes the dimensions of a Czech or Slovak document with a decimal comma (1234,5), as its PDF sheets do, no longer 1234.5.
- The round hole of a worktop, for example for its tap, is a drilling where the hole is, no longer a cut of two arcs across the whole worktop. The production job carries it, and woodWOP refuses it as a hole as deep as the part, saying to export BTLx or the production job, rather than as a through cut. The cut of a sink opening spans the opening, not the whole worktop.
- A connected AI can set the height of ±0,000 when it exports the drawings (`zero_level_mm`, in whole millimetres, or `auto` to find it from the model), as the drawings window can. The document keeps it for the next export.
- A refused BOM, drawing, BTLx, woodWOP or production job export says why: the model changed since it was evaluated, a body did not pass validation, a part is a mesh or has a cut that could not be described as machining, a copy is scaled, two pieces share a code, or a value is out of range, with what to do. It no longer says "incomplete, stale, invalid, or lacks manufacturing semantics" for every cause, and an empty production scope or a package with no drilled piece is no longer told the document "contains no supported visible body geometry".
- Clicking a document in **File > Open Recent** no longer freezes the window while a server that does not answer times out. A document the menu already shows as out of reach is said so at once; any other is checked in the background and opens when its folder answers.
- On macOS and Linux, a settings file with a damaged line no longer loses the language, Open Recent and a newer Kečup's settings at the next save: it is kept whole as `settings.txt.bad` and its readable lines are carried over. A settings file that cannot be read at all is not written over, and the save says why. Two windows saving at once no longer lose each other's change; the file is on the disk before it replaces the old one, is readable only by you, stays a symlink if it is one, and leaves no `.part` file behind.
- The desktop and Android Viewer's settings file follows the same rules: a damaged line no longer resets the language, the damaged file is kept whole as `viewer-settings.txt.bad`, a newer Viewer's settings survive a change made in an older one, a file that cannot be read is not written over and the change says why, and two Viewers saving at once take turns.
- **Open Recent** drops a document whose folder was deleted on this computer, instead of keeping it as out of reach for ever; one on a drive, share or volume that is not there now stays listed.
- **File > New** keeps the window's theme, no longer going back to Graphite, and keeps the Shortcuts window open if it was.
- Dragging the edge of the dock no longer makes a long program on the **Program** page stutter: each frame lays out only the lines in view. A screen reader can read every line of the program, not only those on screen.
- Ketchup and the Viewer keep a log file of the problems they set aside without stopping you, such as a refused import or a connection that hung up: `%LOCALAPPDATA%\Ketchup\ketchup.log` on Windows (`Ketchup Viewer\viewer.log` for the Viewer), `~/Library/Logs/Ketchup` on macOS and `~/.local/state/Ketchup` on Linux. Before, these lines went to a console the window does not have and were lost.
- Data taken from a drawing into the program is refused, with why, when the names of the program library cannot be read, instead of being written without checking that it takes no library name such as `range` over.
- The Android Viewer updates from one release candidate to the next (`-rc.1` to `-rc.2`) and from a development build to a release candidate; before, they had one version code and Android refused the update. The Mac apps' bundle version grows the same way.
- On Linux, a connected AI can no longer have Ketchup read or write files on the Windows drives of WSL (`/mnt/c`, which can be a mapped network share), a virtual machine's host folder (`9p`, `virtiofs`) or a cluster file system (`gpfs`), as for other network file systems. Choose a folder in the Linux file system instead.
- The signed Windows installer signs every program and library it installs, also one that the build computer already trusted through a Windows catalog, which your computer does not have.

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
