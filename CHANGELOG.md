# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org).

<!--
Note: In this file, do not use the hard wrap in the middle of a sentence for compatibility with GitHub comment style markdown rendering.
-->

## [Unreleased]
## [0.0.12] - 2024-10-20

- fix bug: if CacheLine number is greater than 255, the lru will not work correctly
- bump deps

## [0.0.11] - 2024-08-06

- doc

## [0.0.10] - 2024-08-06

- fix doc mistakes

## [0.0.9] - 2024-08-02

- fix: Default for empty cache if loading fails

## [0.0.8] - 2024-08-02

- Default needed as fallback for `get` and `get_mut` if loading fails

## [0.0.7] - 2024-08-01

- improve documentation

## [0.0.6] - 2024-08-01

- improve documentation

## [0.0.5] - 2024-08-01

- fix: (bug) lock failure does not prevent state updating

## [0.0.4] - 2024-07-31

- removed unnecessary atomics

## [0.0.3] - 2024-07-30

- remove Default trait bound for `get` and `get_mut`

## [0.0.2] - 2024-07-29

- Remove `Cacheable` impl for number types
- Spell and doc mistakes fixed

## [0.0.1] - 2024-07-29

- fix: concurrent bugs, tested with `loom`.
- performance improvement

## [0.0.1-alpha2] - 2024-07-27

- `get` and `get_mut` both only need `&Cache` now. Return `RwLockReadGuard` and `RwLockWriteGuard` wrapper respectively.

## [0.0.1-alpha1] - 2024-07-26

- MVP
