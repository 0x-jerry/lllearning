// Cuts a release: bumps the version in Cargo.toml, commits it, and tags it.
//
//   bun run release patch      # 0.1.0 -> 0.1.1
//   bun run release minor      # 0.1.0 -> 0.2.0
//   bun run release major      # 0.1.0 -> 1.0.0
//   bun run release 0.3.1      # an explicit version
//   bun run release patch --push
//
// The tag is what `.github/workflows/release.yml` runs on, and that job refuses
// to publish unless the tag is `v` + the version in Cargo.toml — so a release
// starts here: the bump and the tag are made together, and pushing stays a
// separate step unless `--push` asks for it.

import { $, TOML } from "bun";

const CARGO_TOML = "Cargo.toml";
const CARGO_LOCK = "Cargo.lock";

/** The part of `Cargo.toml` a release reads. */
type Manifest = { package?: { version?: string } };

const argv = process.argv.slice(2);
const push = argv.includes("--push");
const requested = argv.find((arg) => !arg.startsWith("--"));

if (requested === undefined) {
  fail("say how much to bump: patch, minor, major, or a version like 1.2.3");
}

const manifest = await Bun.file(CARGO_TOML).text();
const parsed = TOML.parse(manifest) as Manifest;
const packageTable = parsed.package;
if (packageTable?.version === undefined) fail(`${CARGO_TOML} has no [package] version`);

const current = packageTable.version;
const next = bump(current, requested);
const tag = `v${next}`;

// A release commit that carries anyone else's work is a release nobody can
// reproduce, and a checkout with changes would put them in the bump's commit.
const repo = await $`git rev-parse --is-inside-work-tree`.quiet().nothrow();
if (repo.exitCode !== 0) fail("not a git repository");

const dirty = (await $`git status --porcelain`.quiet().text()).trim();
if (dirty !== "") fail(`the working tree has changes:\n${dirty}`);

const taken = (await $`git tag --list ${tag}`.quiet().text()).trim();
if (taken !== "") fail(`${tag} is already tagged; bump past it`);

const branch = (await $`git rev-parse --abbrev-ref HEAD`.quiet().text()).trim();

// The lockfile records the package's own version too. `cargo metadata` rewrites
// it from the manifest, which is the one place that knows how a lock is encoded.
//
// `TOML.stringify` renders the manifest in its own layout — tables in document
// order, inline tables expanded, LF — so a bump rewrites the file rather than one
// line of it. Parse both sides and the document is identical apart from the
// version; what changes is the shape, once.
packageTable.version = next;
await Bun.write(CARGO_TOML, TOML.stringify(parsed));

const refreshed = await $`cargo metadata --format-version 1`.quiet().nothrow();
if (refreshed.exitCode !== 0) {
  await $`git checkout -- ${CARGO_TOML}`.quiet();
  fail(`cargo could not refresh ${CARGO_LOCK}, so ${CARGO_TOML} was put back:\n${refreshed.stderr.toString()}`);
}

await $`git add ${CARGO_TOML} ${CARGO_LOCK}`.quiet();
await $`git commit -m ${`chore(release): ${tag}`}`.quiet();
await $`git tag -a ${tag} -m ${`lllearning ${tag}`}`.quiet();

console.log(`${tag} on ${branch}: ${current} -> ${next}`);
console.log(`  installer  lllearning_${next}_x64-setup.exe`);
console.log(`  portable   lllearning-${next}-portable.zip`);

if (push) {
  // Only the refs this run made: `--follow-tags` would also send every older
  // annotated tag the pushed commits happen to reach.
  const pushed = await $`git push origin HEAD ${tag}`.nothrow();
  if (pushed.exitCode !== 0) fail("the tag is made but the push failed; push it by hand");
  console.log("pushed: the workflow builds both artefacts and publishes the release");
} else {
  console.log(`  git push origin HEAD ${tag}   # the commit, the tag, and the release`);
}

/** The next version, from `patch`/`minor`/`major` or an explicit one. */
function bump(current: string, how: string): string {
  if (how !== "patch" && how !== "minor" && how !== "major") {
    if (!/^\d+\.\d+\.\d+$/.test(how)) {
      fail(`expected patch, minor, major or a version like 1.2.3, not "${how}"`);
    }
    // A release that goes backwards would publish over a tag the workflow may
    // already have built, so it is a typo rather than an intention.
    if (compare(how, current) <= 0) fail(`${how} is not newer than ${current}`);
    return how;
  }

  const [major = 0, minor = 0, patch = 0] = current.split(".").map(Number);
  switch (how) {
    case "major":
      return `${major + 1}.0.0`;
    case "minor":
      return `${major}.${minor + 1}.0`;
    default:
      return `${major}.${minor}.${patch + 1}`;
  }
}

/** Ordering for two `x.y.z` strings. */
function compare(a: string, b: string): number {
  const left = a.split(".").map(Number);
  const right = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    const difference = (left[i] ?? 0) - (right[i] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

function fail(message: string): never {
  console.error(`release: ${message}`);
  process.exit(1);
}
