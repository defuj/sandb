# Changelog

All notable changes to sandb. Versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.4.1] - 2026-11-05

- The Rust crate version now agrees with the app version. Before, the crate showed 0.1.0.

## [0.4.0] - 2026-11-05

- The results grid has no "Load more" button now. When you scroll near the last row, the grid gets the next rows. The footer shows the row count and the time of the last run, as before. While rows load, the footer shows "Loading more…". With pending edits, the grid does not load more rows.
- A double-click on a connection in the connection tree does not open "Edit connection" now. To edit a connection, use the "…" menu or right-click the connection.
- The connection tree puts tables, views and PostgreSQL materialized views in separate folders. Each folder shows its count. The Tables folder starts open. The other folders start closed. Before, all objects were in one long list, and materialized views showed as views.
- Each folder shows 100 objects. Click "Load 100 more" at the end of the folder to show the next 100. The row also shows how many objects are visible, for example "100 of 1235".
- PostgreSQL: the connection tree hides partitions. The parent table stays in the Tables folder. At the end of the folder, click "Show N partitions" to show them. To hide them again, put the pointer on the Tables folder and click the partition button. The choice applies to all connections and stays after a restart. The tooltip of a partition shows its parent table.
- Quick search (⌘K) now finds tables in all databases that are open in the connection tree. Before, it found only the tables of the active tab's database. Each table shows its connection and database. Press Enter to open the table in its own connection. The active database still gives all its tables, also the tables that you did not open in the tree.
- Fixed: "Drop materialized view…" now runs `DROP MATERIALIZED VIEW`. Before, it ran `DROP VIEW`, and PostgreSQL refused it. The assistant also sees materialized views as "(materialized view)" in its schema summary.
- The assistant can use more than one database in one chat. The databases can be on different connections. Above the chat box, each database of the chat shows as a chip. Click "+ Database" to add or remove databases. You can choose up to 8.
- A new chat starts with the database of the active tab. While the chat is empty, this database changes when you change the tab. When you add or remove a database, the chat keeps your choice. You can change the databases at any time. The change applies to the next question.
- With more than one database, the assistant names the database in each tool step. Each query runs on one database. The assistant cannot join tables of two databases in one SQL statement. It queries each database and then compares the results in its answer.
- Each step shows its database. "Put in editor", "Open in the results grid" and "Run again" use the database of that step, not the database of the active tab.
- Write mode stays a setting of each connection. With more than one connection in a chat, "Read-only" asks which connection can get writes.
- Chat history now shows all chats, not only the chats of the active connection. Each chat shows its databases. Chats from older versions keep their connection.
- PostgreSQL: the results grid, CSV export and the assistant show PostGIS `geometry` and `geography` values as WKT, as `ST_AsText` does, for example `MULTIPOLYGON(((130.8 -3.4,…)))`. Before, they showed as hex EWKB. The grid formats only the first 4 KB of a large geometry, so large polygons do not make the grid slow. SQL dumps keep the EWKB.
- When you edit a geometry cell, sandb keeps the SRID of the column. It uses the SRID of a typed column, for example `geometry(MultiPolygon,4326)`. For a plain `geometry` column, it uses the SRID of the old value. If you type EWKT with `SRID=…;`, sandb uses that SRID.
- A text cell that the grid shows cut off (it ends with "…") can now be edited. When you start the edit, sandb loads the full value of that one cell by its primary key. Before, the grid showed "Truncated text cells are read-only". If you save the full value without a change, the row does not change.
- The app has a new icon: an amber spark on a dark square. The title bar shows the same spark next to "sandb". Before, it showed a database icon.
- Fixed: when the assistant panel is wide, the Table Properties tables now scroll to the right. Before, the last columns were cut off and you could not see them. The assistant panel also becomes narrower when the window has no space for it. Before, the right part of the panel went out of the window.
- Fixed: text fields no longer change what you type. Before, macOS made "paid" into "Paid" in the filter value. Now the filter values, cell editors, search boxes, names, connection fields, Settings fields and the chat box have no automatic capitals, no autocorrection and no spell check.
- The assistant can make charts. It runs one read-only query and shows the result in the chat as a bar, line, area, pie or scatter chart. A chart can show more than one series and stacked series. Charts use the colors of the app theme and keep up to 1,000 rows. They stay in the chat history.
- Charts also work with the "schema" privacy setting. The chart shows the real values to you, but the model gets only the column names and the row count.
- Ask the assistant for a report, and it writes one document with a title, a summary, sections, charts and tables of key figures.
- Each assistant reply has an "Export PDF" button with a text label. The PDF has the reply text and its charts in the order of the chat, on A4 pages with page numbers. Charts stay sharp at any zoom. The desktop app asks where to save the file. In the browser, the file downloads. When you ask for a PDF, the assistant tells you to use this button.
- Fixed: while the assistant writes its text, the chat shows "Writing…". Before, the chat showed no sign of work after a result table.
- Fixed: Markdown tables in assistant replies now fill the width of the chat panel. A wide table scrolls.
- The assistant now knows the date of your computer. The prompt gives today's date, the weekday and the time zone. When you ask about "last month" or "this year", the assistant starts from that date, writes the exact date range in the SQL and tells you the range.
- The PDF uses the built-in PDF fonts. They show Latin text only. Other characters, for example Chinese or Japanese, show as "?".
- In a table Data view, each foreign key cell shows an arrow icon. Click the icon to open the referenced table with a filter for that row. If a tab already shows that table, that tab gets the filter and loads again. NULL values and unsaved changes do not show the icon.
- Table Properties has an "ER Diagram" section, as in DBeaver. It shows the table in the middle, the tables that refer to it on the left and the tables that it refers to on the right. Each box lists the columns with their types and marks primary key and foreign key columns. Lines connect the foreign key columns to the columns that they refer to. Put the pointer on a line to see the foreign key name and its columns. Drag a box header to move the box. "Reset layout" puts all boxes back. Double-click a related table to open it. Use the zoom buttons, ⌘/Ctrl + scroll or a trackpad pinch to zoom from 25% to 200%. Click the percentage to go back to 100%. "Fit to view" makes the full diagram visible.
- Saved queries. Press ⌘S (Ctrl+S) in the editor to save the query with a name. The query keeps its connection and database. After the first save, ⌘S saves your changes at once. ⇧⌘S saves a copy as a new query.
- The sidebar has a "Connections | Queries" switch. The Queries list groups saved queries by connection and shows the name, the start of the SQL and the time of the last save. Type in the filter box to find a query by name or SQL.
- Click a saved query to open it. If a tab already shows the query, that tab comes to the front. Right-click a query, or use its "…" button, to open and run it, rename it (F2), duplicate it, copy its SQL or delete it. Undo in the notice brings back a deleted query.
- A tab with unsaved changes to a saved query shows a dot in place of its X. When you close that tab, sandb asks first. The toolbar shows "Saved" when the tab has no changes. Rename a saved tab with a double-click, and the saved query gets the new name too.
- The command palette (⌘K) lists the saved queries and has a "Save query" action. Saved queries stay after the app restarts.
- Drag a results-grid column border to resize it. For keyboard resizing, focus the border and press the Left or Right arrow key.
- Fixed: opening a cell editor no longer changes the row height or column widths.
- Fixed: on a live connection, the first double-click edit no longer resets the column widths that you set.
- Fixed: an assistant read now shows all the rows that the backend sends (up to 50), not only the first 10. The table scrolls and its header stays visible.
- The assistant now looks at the schema first. Before it queries a table, it calls `describe_table` for that table, and it calls `list_tables` when it does not know the table.
- Assistant tool steps show as a numbered timeline with a status and a duration for each step. A step closes when it finishes. Click a step to open it. Running, waiting and failed steps stay open, and so does the last result.
- Each step has "Call details": the tool name, the call id, the JSON arguments, the result, the duration and the error code.
- Read steps have actions: copy the SQL, put it in the editor, open it in the results grid in a new tab, run it again, and copy the rows as CSV.
- Assistant result tables align numbers to the right and show NULL as the grid does. Click a cell to see its full value. A note tells you when more rows exist.
- Assistant composer: the text box grows as you type. Up in an empty box brings back your last question. Each reply has Copy and Ask again. The suggestions use the tables of the current connection.
- Double-click a cell in a table Data view or a simple query result to edit it. Save applies all row changes in one transaction. Live connections use primary keys and bound values.
- Click a column header to sort the full result before the row limit. Refresh and Load more keep the sort.
- Table Data views now include a filter bar. Add filters by column, operator and value, then apply them together. Refresh, Load more and CSV export keep the active filters.
- Assistant replies now render Markdown tables, lists, headings, links and code blocks. Unsafe HTML is removed. The AI prompt also tells the model not to repeat rows that the query result table already shows.
- Security: the desktop app now keeps connection passwords and the AI API key in the credential store of the operating system. This is the Keychain on macOS, the Credential Manager on Windows and the Secret Service (GNOME Keyring or KWallet) on Linux. Before, the app kept them in `credentials.enc`, and the key for that file was in the app. On the first start, the app moves the passwords from `credentials.enc` to the credential store and then removes the file. If the move fails, the file stays and the app tries again at the next start. On Linux, a Secret Service must run. macOS can ask you once to let sandb use the Keychain.
- Table Properties can now change the table structure. Each section has an add button: Add column, Add constraint (unique, primary key or check), Add foreign key and Add index. The dialog shows the SQL before it runs. Hover a row and click the trash button to drop that column, constraint, foreign key or index. The change commits at once. On a production connection, you must first select a checkbox. Read-only connections and views do not show these buttons. SQLite cannot add or drop constraints and foreign keys on an existing table, so these two sections show a note.
- Table Properties can now edit a column, constraint, foreign key or index. Hover a row and click the pencil button. The dialog opens with the current values, and it shows the SQL before it runs. If nothing changed, Save stays off. A name-only change uses a rename. Other changes use `ALTER COLUMN` (PG), `CHANGE COLUMN` (MySQL) or a drop and add in one statement. MySQL cannot drop and add a foreign key with the same name in one statement, so the change uses two statements and a temporary name. On PG and SQLite, a change with more than one statement runs in one transaction. Limits: SQLite can only rename a column and can only edit a plain index. MySQL always names the primary key PRIMARY. An index with expressions, a sort order, WHERE, INCLUDE or FULLTEXT can only get a new name. MySQL `CHANGE COLUMN` removes AUTO_INCREMENT, COMMENT and ON UPDATE, and the dialog shows a warning about this. PG exclusion constraints have no edit button.
- The AI provider setting now has two options: "OpenAI-compatible" and "Anthropic-compatible". The Ollama, LM Studio, OpenRouter, OpenAI and Custom presets are gone. These servers all use the OpenAI format, so use "OpenAI-compatible" with their base URL. Anthropic-compatible uses `POST /messages` with the `x-api-key` and `anthropic-version` headers. Test uses `GET /models`. When you change the option, a base URL and model that you typed stay. Saved settings with an old preset load as OpenAI-compatible, with the same URL, model and key.

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
