# HLS identity AES-128 decryption

Exact upstream baseline and source target remain
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19). The official checkout
contains both baseline and target; the report at this exact SHA has no intervening
commits, changed files, affected components or unmapped files. No baseline advance
or full HLS compatibility claim is made.

Reviewed `yt_dlp/downloader/hls.py` KEY/MAP/media-sequence construction around
233–292 and `yt_dlp/downloader/fragment.py::decrypter` around 338–365, including
URL-key caching, explicit/implicit IV handling and PKCS7 removal. Mapped source
also contains SAMPLE-AES/DRM, key overrides, external downloaders, live reloads,
and discontinuities that remain outside the native implementation. `test/test_aes.py::TestAES::test_cbc_decrypt` and its fixed CBC vector were
reviewed and ported as a native regression alongside the existing HLS parser
examples.

Native parsing snapshots `HlsAes128 { key_url, iv }` on each fragment. Only raw
16-byte identity AES-128 CBC is supported, with explicit hexadecimal IVs or a
resolved media-sequence IV, distinct-URI key rotation and METHOD=NONE. Encrypted
MAP requires an explicit IV. These rules also follow
[RFC 8216 §§4.3.2.4, 4.3.2.5 and 5.2](https://www.rfc-editor.org/rfc/rfc8216.html).
Only media segments consume sequence numbers. Pinned upstream increments for MAP;
that known difference is documented rather than hidden in normalization. MAP
comparison fixtures use explicit IVs. The native counter is bounded to u64 and
checked against overflow instead of wrapping.

The downloader retrieves and validates keys only when executing the selected
plan. Keys are cached by URL for this invocation (as upstream does); a repeated
URI is assumed to identify the same key throughout that call. Metadata contains
no key bytes. Key responses are limited to exactly 16 identity-encoded bytes;
plaintext-looking binary keys remain valid. Key/segment retries share a fragment
budget, and incomplete/failed plaintext is truncated back to the previous complete
prefix. File publication occurs only after every segment and padding check passes.

RustCrypto aes 0.9 / cbc 0.2 supplies the block cipher and CBC implementation.
The wrapper streams bounded chunks, retains the last ciphertext block and uses
strict library PKCS7 validation. Content lengths/ranges describe ciphertext;
observer stored bytes and successful output sizes describe plaintext. Known
non-block-aligned lengths and invalid padding fail explicitly. No Python,
OpenSSL, FFmpeg or other process is used by runtime decryption.

Regression evidence includes fixed NIST SP800-38A CBC and published RustCrypto
padded vectors, arbitrary chunk boundaries, key caching/rotation/invalid responses,
key and media retries, ciphertext ranges, cancellation and cleanup. Original
OpenSSL-encrypted TS, fMP4 initialization/media and byte-range resources decrypt
byte-for-byte to the original clear media and pass optional FFprobe H.264/AAC
inspection. See [fixture provenance](../../tests/fixtures/hls/README.md).

The existing offline `tools/compare_hls.py` additionally compares actual pinned
HlsFD key URLs and resolved IVs for implicit sequences, explicit IVs, rotation,
METHOD=NONE and encrypted MAP. Its sockets and downloader/key execution remain
blocked; this proves plan agreement on those cases, not upstream runtime decryption
or general encryption/live/DRM parity. The standard test suite uses no live sites.
