# A.1 repository trust proof

Status: passing

## User-visible slice

The desktop uses the operating system directory picker to select a repository. Mars inspects the selection without persistence, then shows a shadcn/ui confirmation dialog containing the canonical repository root and the three capabilities being granted: reading source files, building the local code index, and observing Git metadata. Cancel leaves the profile database unchanged. Confirm persists and opens the project.

Command execution, file writes, network access, and Git mutations are not granted by repository trust.

## Automated evidence

- Rust integration tests cover canonical roots, trust-before-persist, non-Git rejection, malformed `.git` files, linked-worktree `commondir` resolution, repeated-open idempotency, and eight concurrent opens converging on one project.
- Storage migration tests prove the project schema applies once under the bundled SQLite build.
- React tests cover native-picker cancellation, capability preview, trust confirmation, native validation errors, and trust failures remaining visible in the modal.
- Workspace formatting, linting, typechecking, tests, and the production desktop web build pass together.

## Deliberate limits

- The native operating-system picker itself is not driven in the headless UI test environment; the frontend boundary is mocked and the Rust command/domain path is tested separately.
- Project paths currently cross the Tauri command boundary as UTF-8 strings. Arbitrary non-UTF-8 Unix paths are not supported in this slice.
- Restoring the last-opened project and repository removal are later project-lifecycle work.
