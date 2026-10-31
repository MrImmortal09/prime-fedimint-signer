{
  description = "Development environment for prime-fedimint-signer";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            # Rust toolchain
            rustc
            cargo
            rustfmt
            clippy
            rust-analyzer

            # Native C/C++ & bindgen build tools (for aws-lc-sys, secp256k1-sys, rocksdb, etc.)
            cmake
            clang
            llvmPackages.libclang
            pkg-config
            protobuf
            perl
            go
            git
          ];

          buildInputs = with pkgs; [
            openssl
          ];

          # Required by bindgen (used by aws-lc-sys, rocksdb, etc.)
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          # Keeps fedimint test binaries from growing huge with full debug symbols
          CARGO_PROFILE_DEV_DEBUG = "line-tables-only";

          shellHook = ''
            export CC="${pkgs.clang}/bin/clang"
            export CXX="${pkgs.clang}/bin/clang++"
            echo "=== prime-fedimint-signer devShell ==="
            echo "Commands:"
            echo "  cargo test --workspace                                   # Host unit & compatibility tests"
            echo "  cargo test --release --manifest-path devtools/Cargo.toml # Fedimint golden cross-checks"
            echo "  ./scripts/fedimint-checkout.sh && (cd wallet && cargo test) # Watch-only wallet & federation tests"
          '';
        };
      }
    );
}
