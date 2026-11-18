{
  description = "jump-to-tab: zellij plugin (wasm32-wasip1) — dev shell + package build";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, fenix }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;

      # Stable rust with the wasm32-wasip1 (aka wasm32-wasi) std, so
      # `.cargo/config.toml`'s default `build.target = "wasm32-wasip1"`
      # works for plain `cargo build` in the devshell.
      rustToolchain = system: with fenix.packages.${system}; combine [
        stable.toolchain
        targets.wasm32-wasip1.stable.rust-std
      ];
    in
    {
      # nix develop — build/reload the plugin, run `zellij -l zellij.kdl`
      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = [
              (rustToolchain system)
              pkgs.zellij # run the dev layout: zellij -l zellij.kdl
              pkgs.stdenv.cc # host linker, so `cargo test-host` links in-shell
              # host test builds compile zellij-utils' http chain with
              # vendored_curl (see Cargo.toml): openssl-src and libcurl are
              # built from source — perl is required by the openssl build
              pkgs.perl
              # libz-sys resolves zlib through pkg-config instead of vendoring
              pkgs.pkg-config
              pkgs.zlib
            ];
            RUST_SRC_PATH = "${(rustToolchain system)}/lib/rustlib/src/rust/library";
          };
        });

      # nix build — builds the plugin wasm for wasm32-wasip1 (same target as CI)
      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          pkgsWasm = pkgs.pkgsCross.wasm32-wasip1;
          cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        in
        {
          default = pkgsWasm.rustPlatform.buildRustPackage {
            pname = "jump-to-tab";
            version = cargoToml.package.version;
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            # wasm linker setup needs these until nixpkgs#463720 resolves —
            # same workaround as nixpkgs' own zellij plugin helper
            # (pkgs/by-name/ze/zellij/plugins/rust/default.nix)
            nativeBuildInputs = [ pkgsWasm.lld ];
            env.RUSTFLAGS = "-C linker=wasm-ld";

            # wasm artifacts can't run tests; host tests are `cargo test-host`
            # in the devshell (see .cargo/config.toml)
            doCheck = false;
            dontStrip = true;

            installPhase = ''
              runHook preInstall
              install -Dm644 \
                target/wasm32-wasip1/release/jump_to_tab.wasm \
                $out/jump_to_tab.wasm
              runHook postInstall
            '';

            meta = {
              description = cargoToml.package.description;
              homepage = "https://github.com/sunng87/zellij-jump-to-tab";
            };
          };
        });
    };
}
