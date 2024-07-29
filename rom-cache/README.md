<a name="readme-top"></a>

<!-- PROJECT SHIELDS -->
<!--
*** I'm using markdown "reference style" links for readability.
*** Reference links are enclosed in brackets [ ] instead of parentheses ( ).
*** See the bottom of this document for the declaration of the reference variables
*** for contributors-url, forks-url, etc. This is an optional, concise syntax you may use.
*** https://www.markdownguide.org/basic-syntax/#reference-style-links
-->
[![Contributors][contributors-shield]][contributors-url]
[![Forks][forks-shield]][forks-url]
[![Stargazers][stars-shield]][stars-url]
[![Issues][issues-shield]][issues-url]
[![MIT License][license-shield]][license-url]



<!-- PROJECT LOGO -->
<br />
<div align="center">
<h3 align="center">rom-cache</h3>
  <p align="center">
    A rust crate to cache ROM in memory like CPU caching RAM.
    <br />
    <a href="https://docs.rs/rom_cache"><strong>Explore the docs »</strong></a>
    <br />
    <br />
    <a href="https://github.com/kingwingfly/rom-cache">View Demo</a>
    ·
    <a href="https://github.com/kingwingfly/rom-cache/issues">Report Bug</a>
    ·
    <a href="https://github.com/kingwingfly/rom-cache/issues">Request Feature</a>
  </p>
</div>



<!-- TABLE OF CONTENTS -->
<details>
  <summary>Table of Contents</summary>
  <ol>
    <li><a href="#import">Import</a></li>
    <li>
      <a href="#about-the-project">About The Project</a>
      <ul>
        <li><a href="#built-with">Built With</a></li>
      </ul>
    </li>
    <li><a href="#usage">Usage</a></li>
    <li><a href="#changelog">Changelog</a></li>
    <li><a href="#roadmap">Roadmap</a></li>
    <li><a href="#contributing">Contributing</a></li>
    <li><a href="#license">License</a></li>
    <li><a href="#contact">Contact</a></li>
    <li><a href="#acknowledgments">Acknowledgments</a></li>
  </ol>
</details>

<!-- IMPORT -->
## Import
```toml
[dependencies]
rom_cache = { version = "0.0.1" }
```

<!-- ABOUT THE PROJECT -->
## About The Project

A rust crate to cache ROM in memory like CPU caching RAM.

Trait `Cacheable` is provided to let user define how to `load` and `store` data in Secondary Storage.

`get` and `get_mut` will lock the `CacheGroup`, then load and upgrade LRU.

1. get (RwLockReadGuard)
- cache hit: return `CacheRef` from cache.
- cache busy: `CacheError::Busy`, cannot evict LRU-chosen `CacheLine` since being used.
- cache locked: `CacheError::Locked`, cannot read-lock while writing.

2. get_mut (RwLockWriteGuard)
- cache hit: return `CacheMut` from cache, and dereference `CacheMut` will set `CacheLine` dirty.
- cache busy: `CacheError::Busy`, cannot evict LRU-chosen `CacheLine` since being used.
- cache locked: `CacheError::Locked`, cannot write-lock while reading or writing.

Any **dirty** `CacheLine` will be written back (`Cacheable::store()`) to Secondary Storage when evicted.

### feature

- `nightly`: enable `#![feature(trait_upcasting)]` to simplify the `Cacheable` trait. (Nightly Rust is needed)

<p align="right">(<a href="#readme-top">back to top</a>)</p>



### Built With

* Rust
* Miri (Testing)
* Loom (Concurrency Testing)

<p align="right">(<a href="#readme-top">back to top</a>)</p>


<!-- USAGE EXAMPLES -->
## Usage
### Example

```rust ignore
# use rom_cache::Cache;
// e.g 2-way set associative cache (8 sets/groups)
let cache: Cache<8, 2> = Default::default();
cache.get::<isize>().unwrap();
cache.get::<String>().unwrap();
{
    let mut s = cache.get_mut::<String>().unwrap();
    cache.get::<u64>().unwrap();
    cache.get::<usize>().unwrap();
    *s = "".to_string();    // set dirty
}
{
    let s = cache.get::<String>().unwrap(); // other threads may evict `String` and it's stored,
                                            // this will load it back
    assert_eq!(*s, "");                     // The `load` result is `""`
}
```

_For more examples, please refer to the [Tests](https://github.com/kingwingfly/rom-cache/tree/dev/tests), [Example](https://github.com/kingwingfly/rom-cache/blob/dev/examples/example.rs) or [Documentation](https://docs.rs/rom_cache)_

<p align="right">(<a href="#readme-top">back to top</a>)</p>


<!-- CHANGELOG -->
## Changelog

todo

[more detailed changelog](https://github.com/kingwingfly/rom-cache/blob/dev/CHANGELOG.md)

<p align="right">(<a href="#readme-top">back to top</a>)</p>


<!-- ROADMAP -->
## Roadmap

- [ ] allow concurrent access
- [ ] auto load when cache miss

<!-- CONTRIBUTING -->
## Contributing

Contributions are what make the open source community such an amazing place to learn, inspire, and create. Any contributions you make are **greatly appreciated**.

If you have a suggestion that would make this better, please fork the repo and create a pull request. You can also simply open an issue with the tag "enhancement".
Don't forget to give the project a star! Thanks again!

1. Fork the Project
2. Create your Feature Branch (`git checkout -b feature/AmazingFeature`)
3. Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
4. Push to the Branch (`git push origin feature/AmazingFeature`)
5. Open a Pull Request

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- LICENSE -->
## License

Distributed under the MIT License. See `LICENSE.txt` for more information.

<p align="right">(<a href="#readme-top">back to top</a>)</p>



<!-- CONTACT -->
## Contact

Louis - 836250617@qq.com

Project Link: [https://github.com/kingwingfly/rom-cache](https://github.com/kingwingfly/rom-cache)

<p align="right">(<a href="#readme-top">back to top</a>)</p>




<!-- MARKDOWN LINKS & IMAGES -->
<!-- https://www.markdownguide.org/basic-syntax/#reference-style-links -->
[contributors-shield]: https://img.shields.io/github/contributors/kingwingfly/rom-cache.svg?style=for-the-badge
[contributors-url]: https://github.com/kingwingfly/rom-cache/graphs/contributors
[forks-shield]: https://img.shields.io/github/forks/kingwingfly/rom-cache.svg?style=for-the-badge
[forks-url]: https://github.com/kingwingfly/rom-cache/network/members
[stars-shield]: https://img.shields.io/github/stars/kingwingfly/rom-cache.svg?style=for-the-badge
[stars-url]: https://github.com/kingwingfly/rom-cache/stargazers
[issues-shield]: https://img.shields.io/github/issues/kingwingfly/rom-cache.svg?style=for-the-badge
[issues-url]: https://github.com/kingwingfly/rom-cache/issues
[license-shield]: https://img.shields.io/github/license/kingwingfly/rom-cache.svg?style=for-the-badge
[license-url]: https://github.com/kingwingfly/rom-cache/blob/master/LICENSE.txt
[linkedin-shield]: https://img.shields.io/badge/-LinkedIn-black.svg?style=for-the-badge&logo=linkedin&colorB=555
[product-screenshot]: images/screenshot.png
