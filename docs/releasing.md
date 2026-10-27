# Releasing

How a nibOS release is made, in the order it is made, so that every release goes the same way.
The build half is one script, [`tools/release-build.sh`](../tools/release-build.sh); the rest
needs a person: the key, the GitHub account, a device. 0.1.0-alpha4 (2026-10-27) is the first
release made to this page; alpha1 to alpha3 were made by the same steps by hand, and the traps
below are theirs.

Releases come from the **public trees**: this repository, and beside it `dslot-public`,
`nibcase-public`, `inktick-public`, `DSperate` and `inkdot`. Work done in a private tree is
ported first (`git format-patch` | `git am`), and the public trees are what the images and the
source offer are built from.

## 1. The trees

1. Everything for the release is committed in each tree the images are built from, and pushed
   where it has a remote (the source offer names those commits).
2. `CHANGELOG.md`: the `## Unreleased` section becomes `## <version>`. Commit it.
3. Tag it, annotated: `git tag -a v<version> -m "nibOS <version>: <one line>"`. The images are
   stamped with this commit (`NIBOS_BUILD`), and the script refuses a tag that isn't at HEAD.

## 2. The build

```
tools/release-build.sh <version>
```

It checks that this tree is clean at the tag and that the project trees are clean checkouts,
then, logged to `out/release-build-<version>.log`:

1. Rebuilds `nibos-hw` in the VM (`hw/out`). `SKIP_BUILD=1` builds take the newest package
   there, so without this a change under `hw/` would not ship (0.1.0-alpha2's `nib update`
   lacked its bad-root refusal for that reason).
2. Builds both images, `VERSION=<version> SKIP_BUILD=1 tools/make-image.sh` and `--dslot`, from
   the public trees (`DSLOT_SRC`, `NIBCASE_SRC`, `INKTICK_SRC`; alpha4's first build took the
   private `nibcase` beside the repository and had to be thrown away).
3. Runs `tools/test-image.sh` on both images, then `tools/test-update.sh` on each, one at a
   time: they share the VM's loop devices, and two at once fail with "Device or resource busy".
4. Collects the corresponding source from `out/sources-cache` (shared by every release; only
   what changed is fetched) into `out/sources-<version>` and packs it. It wants
   `out/sources-<version>/SOURCES.md` first: copy the previous release's, change the version,
   and read it through for anything else that changed (the DSlot version, the projects' commits).

Kernel and U-Boot are not rebuilt here: a release that changes them is built once without
`SKIP_BUILD` first ([building.md](building.md)).

## 3. Signing and publishing

1. Release notes: `out/release-notes-<version>.md`, from the previous release's. What changed,
   updating from the earlier releases, installing fresh, the key, known issues.
2. Sign, with the update key (minisign Ed25519, key ID `41B02B0D6ED49D26`; the passphrase is
   asked for):

   ```
   tools/make-release.sh --key ~/.minisign/nibos-update.key --channel stable \
     --notes out/release-notes-<version>.md out/work-nibos-<version>-rgds out/work-nibos-<version>-rgds-dslot
   ```

   The alphas are all on the `stable` channel. It writes `out/release-<version>/`: for each
   edition the `.img.xz`, `.manifest.json`, `.minisig` and `.slot`, then `SHA256SUMS`.
3. Push this tree and the tag (`git push origin main v<version>`), and the project trees.
4. The GitHub release, named `nibOS <version>`, not a prerelease:

   ```
   gh release create v<version> --verify-tag --title "nibOS <version>" \
     --notes-file out/release-notes-<version>.md out/release-<version>/*
   gh release upload v<version> out/sources-<version>/nibos-<version>-sources-*.tar.zst \
     out/sources-<version>/nibos-<version>-sources-manifest.tsv out/sources-<version>/SOURCES.md
   ```

   A device's updater reads the manifests from here, so the release is live for every device
   the moment it exists.

## 4. Checking it

1. The OTA, on a device on the previous release: `nib update check`, `nib update download`
   (about 370 MB; a few minutes), `nib update apply`, and the restart. `nib update`'s second
   line is the status; the notes follow it, so never search the whole output for a word. After
   the restart: the new version in `/usr/lib/nibos/release`, `nibos-boot-good` confirmed, the
   root check clean, and whatever the release changed.
2. The site (`nibos-site-public`): the release block in `zola.toml` (version, tag, date,
   commit, each image's size and sha256 from `SHA256SUMS`), `content/download.md` (which
   alpha, which releases it updates from, the release notes), and any manual page the release
   changes. Push, and check the live page.

## What not to do

- Don't force-move a tag; make a new release instead. (The tool that helps here is refused a
  forced push anyway.)
- Don't run two of the VM's image jobs at once.
- Don't build the DSlot edition with a private tree beside the repository standing in for a
  public one; the source offer must name public commits.
