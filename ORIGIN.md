# Origin of `tests/gnu_patch_testsuite`

The files in `tests/gnu_patch_testsuite/` are the `tests/` directory from
GNU patch 2.8, copied unchanged (including `Makefile.am`, `Makefile.in`,
`test-lib.sh`, and every test script).

- Upstream: <https://ftp.gnu.org/gnu/patch/patch-2.8.tar.gz>
- Released: 29 March 2025
- SHA-256: `308a4983ff324521b9b21310bfc2398ca861798f02307c79eb99bb0e0d2bf980`
- Signature: `patch-2.8.tar.gz.sig` from the same directory
- Verification: GnuPG reported a good signature from Andreas Gruenbacher
  `<agruen@gnu.org>` (RSA key `259B3792B3D6D319212CC4DCD5BF9FEB0313653A`,
  fingerprint `259B 3792 B3D6 D319 212C  C4DC D5BF 9FEB 0313 653A`).
  The key in the GNU keyring (`https://ftp.gnu.org/gnu/gnu-keyring.gpg`)
  is expired; the signature itself verifies.

`tests/run_gnu_testsuite.sh` runs these scripts against this project's
`patch` binary. It does not modify the imported files.
