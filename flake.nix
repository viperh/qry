{
  description = "qry: a terminal SQL client for PostgreSQL, MySQL, MariaDB, SQLite and Oracle";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        version = cargoToml.workspace.package.version;

        qry = pkgs.rustPlatform.buildRustPackage {
          pname = "qry";
          inherit version;
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          cargoBuildFlags = [ "-p" "qry" ];

          nativeBuildInputs = [ pkgs.installShellFiles ];

          # CI runs the tests (several need database servers, and the
          # snapshot tests expect the repository layout).
          doCheck = false;

          postInstall = pkgs.lib.optionalString
            (pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform) ''
              $out/bin/qry generate man > qry.1
              installManPage qry.1
              installShellCompletion --cmd qry \
                --bash <($out/bin/qry generate completions bash) \
                --zsh <($out/bin/qry generate completions zsh) \
                --fish <($out/bin/qry generate completions fish)
            '';

          meta = {
            description = "Terminal SQL client for PostgreSQL, MySQL, MariaDB, SQLite and Oracle";
            homepage = "https://github.com/viperh/qry";
            license = pkgs.lib.licenses.mit;
            mainProgram = "qry";
          };
        };
      in
      {
        packages = {
          inherit qry;
          default = qry;
        };

        apps.default = flake-utils.lib.mkApp { drv = qry; } // {
          meta.description = qry.meta.description;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
            python3
            cargo-deny
            cargo-insta
            cargo-fuzz
          ];
          # No test touches the developer's real keychain.
          QRY_KEYRING = "off";
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };

        formatter = pkgs.nixpkgs-fmt;
      });
}
