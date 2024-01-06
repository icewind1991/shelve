{
  config,
  lib,
  pkgs,
  ...
}:
with lib; let
  cfg = config.services.shelve;
in {
  options.services.shelve = {
    enable = mkEnableOption "Enables the shelve service";

    port = mkOption rec {
      type = types.int;
      example = 8080;
      description = "The port to listen on";
    };

    bindAddress = mkOption {
      type = types.str;
      default = "0.0.0.0";
      description = "Address to listen on";
    };

    tokens = mkOption {
      type = types.listOf types.str;
      default = [];
      example = ["foo" "bar"];
      description = "upload tokens";
    };

    basedir = mkOption {
      type = types.str;
      description = "data base directory";
    };

    package = mkOption {
      default = pkgs.shelve;
      type = types.package;
      defaultText = literalExpression "pkgs.shelve";
      description = "package to use";
    };

    openPort = mkOption {
      type = types.bool;
      default = false;
      example = true;
      description = "open port";
    };
  };

  config = mkIf cfg.enable {
    networking.firewall.allowedTCPPorts = lib.optional cfg.openPort cfg.port;

    users.groups.shelve = {};
    users.users.shelve = {
      isSystemUser = true;
      group = "shelve";
    };

    systemd.services.shelve = {
      wantedBy = ["multi-user.target"];
      environment = {
        ROCKET_PORT = toString cfg.port;
        ROCKET_ADDRESS = cfg.bindAddress;
        BASEDIR = cfg.basedir;
        TOKENS = concatStringsSep "," cfg.tokens;
      };
      script = "${cfg.package}/bin/shelve";

      serviceConfig = {
        Restart = "on-failure";
        User = "shelve";
        PrivateTmp = true;
        ProtectSystem = "full";
        ProtectHome = true;
        NoNewPrivileges = true;
        ReadWritePaths = cfg.basedir;
        NoExecPaths = cfg.basedir;
      };
    };
  };
}
