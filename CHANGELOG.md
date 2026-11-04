# Changelog

All notable changes to sandb. Versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.3.0] - 2026-11-04

- A new sidebar button in the title bar, next to the app name, hides or shows the sidebar. The command palette (⌘K) also has a "Toggle sidebar" action. The connection tree keeps its expanded nodes when you hide the sidebar.
- The results area has a tab strip above the grid, as in DBeaver. The tab shows the name of the source table, or "Results". The X on the tab hides the results. This tab replaces the X button at the top-right corner of the grid.
- The line between the editor and the results has two arrow buttons. The up arrow makes the results area as large as possible, and the down arrow makes the editor as large as possible. Click the other arrow to go back to the previous heights.
- Fixed: a query with its own `LIMIT` larger than 200 showed only 200 rows. The run now fetches the number of rows in the `LIMIT`, up to 100,000. Queries without a `LIMIT` still show 200 rows first, and "Load more" fetches more.
- Tables in the connection tree now open in a table tab, as in DBeaver. A single click only selects the table. Double-click, ⌘-click (Ctrl-click on Windows and Linux), Enter or "View data" opens it. If a tab for the table is already open, that tab comes to the front.
- A table tab has two views. "Data" shows the rows of the table and has a Refresh button. "Properties" shows the Columns, Constraints, Foreign Keys, References (foreign keys of other tables that point to this table), Indexes and DDL of the table.
- Fixed: a right-click on an empty part of the app showed the WebView menu (Reload, Play All Animations). Now this menu shows only in text fields and on selected text, for Cut, Copy and Paste.
- Fixed: ⌘W (Ctrl+W on Windows) closed the app window. Now it closes the active tab, as in a browser. The menu item is File > Close Tab. To close the window on macOS, use Window > Close Window (⇧⌘W).
- You can drag the left edge of the assistant panel to change its width (280 to 640 px). The arrow keys also move the edge when it has focus.
- The assistant keeps a history of chats for each connection. The clock button shows the list. Click a chat to open it again, or delete it. The pen button starts a new chat. The history stays after the app restarts.
- The assistant asks for write approval inside the chat, not in a dialog. The card shows the SQL, the connection, the statement type and the approximate number of rows. For a statement that removes data, the card shows a red warning, and Deny has the focus.
- The assistant can ask you a question in the chat. For example, if a table name does not exist, it shows the available tables as buttons. You can click one, type your own answer, or skip the question.
- The assistant now uses your AI server. Before, the chat gave scripted replies and did not send a request to the model. The assistant can list tables, describe a table, and run read-only queries through the AI read guard. It can propose a write, but it does not run it. Approval of AI writes comes in a later phase. The Stop button stops a reply.
- Fixed: the Test button in Settings showed "Connected" for any model name, also a wrong one. Now Test asks the server for its model list (`GET /models`). If the server does not list the model, Test shows "Failed" and the models that the server has.
- Settings saves the AI provider, base URL, model, privacy mode and limits. The desktop app keeps the API key in the encrypted `credentials.enc` file. The dev bridge keeps it in memory.
- With "Schema only", the model gets the column names and the row count of a query, but no row values. Query errors do not go to the model with their database text.

## [0.2.0] - 2026-11-03

- You can drag the edge of the sidebar to change its width.
- You can drag the line between the editor and the results to change their heights.
- A new footer below the editor has a Results button. It shows or hides the results grid. When you run a query, the grid opens.
- A close button (X) at the top-right corner of the results grid hides the grid.
- Right-click in the editor to open a menu with these items:
  - "Run statement at cursor" runs the statement under the cursor. If you selected text, the item is "Run selection".
  - "Run all statements" runs all the statements in the editor.
  - "Run SQL script from file…" runs a `.sql` file. The toolbar shows a progress bar with the number of completed statements.
  - Cut, Copy, Paste and Select all.
- Right-click a table or view in the connection tree to open a table menu with these items:
  - View data, Count rows and Copy name.
  - "Generate DDL", "Generate SELECT" and "Generate INSERT" open the SQL in a new query tab. They do not run it.
  - "Export DDL…" saves the CREATE statements to a `.sql` file. "Export as SQL (DDL + data)…" also adds INSERT statements for all rows. "Export as CSV…" saves all rows. Export needs the desktop app.
  - Refresh reloads the table list.
  - "Truncate…" and "Drop table…" ask you to confirm first. On a production connection, you must also select a checkbox. The statement commits immediately and goes into the audit log. A read-only connection cannot use these items.
- The database list in the editor bar now opens to the right. Before, the sidebar hid its left part.

## [0.1.2] - 2026-11-03

- You can now delete the last connection. When no connection remains, sandb shows the "No connections yet" screen.

## [0.1.1] - 2026-11-03

- Test release for the auto-update flow. No functional changes.

## [0.1.0] - 2026-11-03

First release.

- Connect to PostgreSQL, MySQL, MariaDB and SQLite.
- SQL editor with autocomplete, query tabs and an editable results grid.
- AI chat with any OpenAI-compatible provider, with read-only and write-approval safeguards.
- Saved passwords in an encrypted file in the app data folder.
- Version in Settings, with a check for updates that asks before it installs.
