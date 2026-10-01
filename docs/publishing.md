# Publishing a plugin

`chunkzero/rpp/.github/actions/publish-plugin` packs a plugin when you push a version tag, attaches the archive to
the tag's GitHub release, and opens a pull request that adds the version to
[chunkzero/rpp-registry](https://github.com/chunkzero/rpp-registry).

## Setup

1. Set `name`, `version`, and an `rpp` version range in the plugin's `rpp.json`. Packing fails without `rpp`.
2. Create a personal access token that can fork `chunkzero/rpp-registry` and open pull requests there, and store it
   as the repository secret `RPP_REGISTRY_TOKEN`. `GITHUB_TOKEN` cannot open pull requests in another repository.
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
      - uses: chunkzero/rpp/.github/actions/publish-plugin@v0.5.0
        with:
          rpp-version: "0.5.0"
          registry-token: ${{ secrets.RPP_REGISTRY_TOKEN }}
```

Pin the action to the same rpp release you install. Push a tag `v<version>` that matches the `version` in `rpp.json`
to publish. The first publish of a new plugin name registers it; later publishes append a version.

## Inputs

| Input               | Description                                                                                                                                                                                  |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rpp-version`       | rpp release to install, without the leading `v`. Required.                                                                                                                                   |
| `registry-token`    | Token described above. Required.                                                                                                                                                             |
| `install-command`   | Installs dependencies. Detected from the lockfile when empty: `npm ci`, `pnpm i --frozen-lockfile`, `yarn --immutable`, or `bun i --frozen-lockfile`. Nothing runs without a `package.json`. |
| `working-directory` | Plugin directory, relative to the repository root. Defaults to `.`.                                                                                                                          |

## What it does

1. Installs dependencies and rpp.
2. Runs `rpp plugin pack --json`, and fails unless the tag is `v<version>`.
3. Uploads `<name>-<version>.rpp.tgz` and its `.sha256` to the tag's release, creating the release if needed.
4. Forks the registry, adds the version to `plugins/<name>.json`, regenerates `index.json`, runs
   `registry.py check --base origin/main`, and opens a pull request from the branch `<name>-<version>`.

A plugin's public types must not reference types from its npm dependencies, because declarations from `node_modules`
are not packed.
