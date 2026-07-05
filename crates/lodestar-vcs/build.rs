//! Enlaza `advapi32` en Windows.
//!
//! `libgit2-sys` (libgit2 1.8) usa APIs de Windows para propiedad de ficheros por
//! SID/token de proceso (`git_fs_path_owner_is`), el registro y la CryptoAPI legada
//! (`OpenProcessToken`, `GetNamedSecurityInfoW`, `RegOpenKeyExW`, `CryptGenRandom`…),
//! todas en `advapi32.dll`. La versión 0.17.0+1.8.1 no emite el `-l advapi32` en MSVC,
//! así que el enlazado falla con `LNK2019` (símbolos sin resolver). Lo añadimos aquí,
//! en el crate dueño de git2, para que cualquier binario que enlace `lodestar-vcs`
//! (cli, mcp, desktop y los tests) resuelva esos símbolos. No-op fuera de Windows.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
