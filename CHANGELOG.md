# Changelog

All notable changes to this project will be documented here. Format based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versioning follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [unreleased]

### Added

- A rust stack

### Changed

- Output yield now selects on payment credential, not just address. This makes
  downstream processes easier.
- Make constants persist for settled, and make them a sibling of stage in the
  datum, rather than a field. This makes downstream processes easier.
- Drop support for `ByHash`, change ByAsset to simply `Asset`.
- Other (hopefully all) single variant constr have been flattened to lists.

### Removed

- The entire js stack broken by hardfork.

## [0.0.0] - 2025-06-07

### Added

- Initial release

## [Template]

### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

### Breaking
