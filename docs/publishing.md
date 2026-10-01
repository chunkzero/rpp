# Publishing a plugin

`chunkzero/rpp/.github/actions/publish-plugin` packs a plugin when you push a version tag, attaches the archive to
the tag's GitHub release, and opens a pull request that adds the version to
[chunkzero/rpp-registry](https://github.com/chunkzero/rpp-registry).

## Setup

1. Set `name`, `version`, and an `rpp` version range in the plugin's `rpp.json`. Packing fails without `rpp`.
2. Give the workflow a token that can open pull requests on `chunkzero/rpp-registry`. `GITHUB_TOKEN` cannot open
   pull requests in another repository.
   - **Plugins in the chunkzero organization** use the organization's GitHub App, which is installed on
     `chunkzero/rpp-registry` with Contents and Pull requests write access. The organization secrets `RPP_APP_ID` and
     `RPP_APP_PRIVATE_KEY` are available to every chunkzero repository, and the workflow below exchanges them for a
     short-lived token. The app pushes a branch to the registry directly, and its bot opens the pull request.
   - **Other plugins** use a classic personal access token with the `public_repo` scope, stored as the repository
     secret `RPP_REGISTRY_TOKEN`. The action pushes to the token owner's fork of the registry. Drop the token step
     below and pass `registry-token: ${{ secrets.RPP_REGISTRY_TOKEN }}` instead.
3. Add the workflow below.

```yaml
name: Publish
on:
  push:
    tags: ["v*"]

permissions:
  contents: write

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - id: registry-token
        uses: actions/create-github-app-token@v2
        with:
          app-id: ${{ secrets.RPP_APP_ID }}
          private-key: ${{ secrets.RPP_APP_PRIVATE_KEY }}
          owner: chunkzero
          repositories: rpp-registry
      - uses: chunkzero/rpp/.github/actions/publish-plugin@v0.5.0
        with:
          rpp-version: "0.5.0"
          registry-token: ${{ steps.registry-token.outputs.token }}
```

Pin the action to the same rpp release you install. Push a tag `v<version>` that matches the `version` in `rpp.json`
to publish. The first publish of a new plugin name registers it; later publishes append a version.

## Inputs

| Input               | Description                                                                                                                                                                                  |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rpp-version`       | rpp release to install, without the leading `v`. Required.                                                                                                                                   |
| `registry-token`    | Registry token from step 2. Required.                                                                                                                                                        |
| `install-command`   | Installs dependencies. Detected from the lockfile when empty: `npm ci`, `pnpm i --frozen-lockfile`, `yarn --immutable`, or `bun i --frozen-lockfile`. Nothing runs without a `package.json`. |
| `working-directory` | Plugin directory, relative to the repository root. Defaults to `.`.                                                                                                                          |

## What it does

1. Installs dependencies and rpp.
2. Runs `rpp plugin pack --json`, and fails unless the tag is `v<version>`. The JSON has `file` (the archive's file
   name) and `path` (where it was written under `--out`).
3. Uploads `<name>-<version>.rpp.tgz` and its `.sha256` to the tag's release, creating the release if needed. If the
   release already has that asset, the run fails when its bytes differ and skips the upload when they match, so a
   failed run can be retried.
4. Adds the version to `plugins/<name>.json`, regenerates `index.json`, and runs `registry.py check --base origin/main`,
   which downloads the uploaded archive and verifies its hash. A version already in the registry fails here.
5. Pushes the branch `<name>-<version>` to the registry, or to the token owner's fork when the token cannot push there,
   and opens a pull request.

A plugin's public types must not reference types from its npm dependencies, because declarations from `node_modules`
are not packed.
