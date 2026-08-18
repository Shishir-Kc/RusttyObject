# RusttyObject

RusttyObject turns a GitHub repository into object storage. Directories are buckets, files are objects, and every write is committed to GitHub.

## What is connected

- `RusttyObject/RusttyObject` is the Rust API server and CLI.
- `RusttyObject.Desktop` is the React/Vite console.
- GitHub OAuth authenticates the browser and keeps the GitHub access token server-side in a short-lived in-memory session.
- The console lists repositories, indexes the selected branch, derives buckets from file paths, and uploads files through the GitHub Contents API.
- Public object previews use each file's direct `https://raw.githubusercontent.com/...` URL. Private repositories use the authenticated preview route so their GitHub contents are not exposed.
- The repository switcher remembers the last four repositories visited. The notification bell reads GitHub notifications, and the profile card loads the account avatar, contribution count, repository totals, and recent activity.

## GitHub OAuth setup

Create an OAuth App in GitHub with this callback URL:

```text
http://localhost:8787/auth/github/callback
```

Copy the backend environment template and fill in the GitHub OAuth values:

```bash
cd RusttyObject/RusttyObject
cp .env.example .env
```

The Rust server loads `.env` automatically. Start the API with:

```bash
cargo run -- server
```

To start both the Rust backend and the Vite console with one command, run this from the project root:

```bash
./dev.sh
```

The script starts the API at `http://localhost:8787` and the console at `http://localhost:5173`. Press `Ctrl+C` to stop both processes. You can also run them separately if needed.

To make the command available from any directory for your user, install the launcher once from the project root:

```bash
install -m 755 ./rustty ~/.local/bin/rustty
```

For all users on the machine, use `/usr/local/bin` instead:

```bash
sudo install -m 755 ./rustty /usr/local/bin/rustty
```

After that, run `rustty` from any directory. Set `RUSTTYOBJECT_HOME` if the project is moved to another location.

To start only the console in another terminal:

```bash
cd RusttyObject.Desktop
bun run dev
```

Open `http://localhost:5173`, sign in with GitHub, choose a repository, and upload an object. The upload is committed to the selected repository's default branch.

## CLI

From any folder that should become an object workspace:

```bash
rusttyobject init --repo owner/name
```

This creates `config.rustyobject` in that folder and indexes every file below it, excluding `.git`, `target`, `node_modules`, `.next`, `dist`, and the index itself. The file contains the repository, branch, relative paths, byte sizes, MIME types, and SHA-256 hashes. Older `.rustyobject` files are still accepted when rebuilding or pushing an index.

Refresh the local index after files change:

```bash
rusttyobject index
```

To push the indexed workspace from the CLI, provide a GitHub personal access token with repository contents write access:

```bash
export GITHUB_TOKEN="github_pat_..."
rusttyobject push
```

The server and CLI intentionally use separate credentials: browser OAuth sessions are never written into `config.rustyobject` or exposed to the frontend.

## API surface

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/auth/github` | Begin OAuth |
| `GET` | `/auth/github/callback` | Finish OAuth and set the session cookie |
| `GET` | `/api/session` | Get the current signed-in user |
| `GET` | `/api/repositories` | List repositories available to the user |
| `GET` | `/api/profile` | Load GitHub profile stats and recent activity |
| `GET` | `/api/notifications` | Load unread GitHub notifications |
| `GET` | `/api/repositories/:owner/:repo/objects` | Index the selected branch |
| `GET` | `/api/repositories/:owner/:repo/file?path=...` | Authenticated inline preview/download |
| `POST` | `/api/repositories/:owner/:repo/files` | Commit a multipart upload to GitHub |
| `POST` | `/api/auth/logout` | End the browser session |

Uploads accept `file`, `bucket`, `branch`, `path`, and `message` multipart fields. Files are limited to 100 MB by the GitHub Contents API path used here.

## Production notes

The in-memory session store is suitable for local development and a single process. Before deploying multiple instances, replace it with a shared session store, set secure cookies behind HTTPS, and move OAuth secrets to a secret manager. Large media objects should use Git LFS or a dedicated blob transfer path instead of the Contents API. If the app was authorized before notifications were added, sign out and sign in again once so GitHub grants the notification scope.
