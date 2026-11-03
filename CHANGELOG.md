# Changelog

All notable changes to sandb. Versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
