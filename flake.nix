{
  description = "Dev shell for building the jump-to-tab zellij plugin (wasm32-wasip1)";

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
      forAllSystems = f: nixpkgs.lib.genAttrs systems f;
    in {
      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          # stable rust toolchain with the wasm32-wasip1 (aka wasm32-wasi) std
          toolchain = with fenix.packages.${system};
            combine [
              stable.toolchain
              targets.wasm32-wasip1.stable.rust-std
            ];
        in {
          default = pkgs.mkShell {
            packages = [ toolchain ];
          };
        });
    };
}
