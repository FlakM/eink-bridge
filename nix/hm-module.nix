self:
{ config, lib, pkgs, ... }:
let
  cfg = config.programs.eink-bridge;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};

  # One link per entry, so files a user keeps next to them (a hand-written agent in
  # ~/.claude/agents) are left alone.
  linkEach = dir: target:
    lib.mapAttrs' (name: _: lib.nameValuePair "${target}/${name}" {
      source = "${dir}/${name}";
      force = true;
    }) (builtins.readDir dir);
in
{
  options.programs.eink-bridge = {
    enable = lib.mkEnableOption "eink-bridge: the review CLI, the agent tools and the channel";

    package = lib.mkOption {
      type = lib.types.package;
      default = packages.eink-bridge;
      description = "Server and `eink-review` CLI.";
    };

    toolsPackage = lib.mkOption {
      type = lib.types.package;
      default = packages.harness-tools;
      description = "The commands the skills run (`eink-pr-review`, `eink-watch-feedback`, ...).";
    };

    channelPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = packages.einkChannel;
      description = "The Claude Code MCP channel (`eink-channel-server`); null to leave it out.";
    };

    harnessPackage = lib.mkOption {
      type = lib.types.package;
      default = packages.harness;
      description = "Skills, output styles and agents.";
    };

    claude.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Link the skills, output styles and agents into ~/.claude.";
    };

    server = {
      enable = lib.mkEnableOption "the `eink-serve` user service";

      environment = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = { };
        example = { EINK_OCR_OPENAI_ENABLED = "true"; };
        description = "Extra environment for `eink-serve` (see docs/setup.md).";
      };

      environmentFiles = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Files with secrets for `eink-serve`, kept out of the store.";
      };

      path = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = (config.home.sessionPath or [ ]) ++ [
          "/etc/profiles/per-user/${config.home.username}/bin"
          "/run/wrappers/bin"
          "/nix/var/nix/profiles/default/bin"
          "/run/current-system/sw/bin"
        ];
        description = "PATH for `eink-serve`: the OCR and voice tools it starts are looked up here.";
      };
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    {
      home.packages = [ cfg.package cfg.toolsPackage ] ++ lib.optional (cfg.channelPackage != null) cfg.channelPackage;
    }

    (lib.mkIf cfg.claude.enable {
      home.file = linkEach "${cfg.harnessPackage}/skills" ".claude/skills"
        // linkEach "${cfg.harnessPackage}/output-styles" ".claude/output-styles"
        // linkEach "${cfg.harnessPackage}/agents" ".claude/agents";
    })

    (lib.mkIf (cfg.server.enable && pkgs.stdenv.hostPlatform.isLinux) {
      systemd.user.services.eink-serve = {
        Unit = {
          Description = "eink-bridge review server";
          After = [ "network-online.target" ];
        };
        Service = {
          ExecStart = "${cfg.package}/bin/eink-serve";
          Restart = "on-failure";
          RestartSec = 5;
          Environment = [ "RUST_LOG=info" "PATH=${lib.concatStringsSep ":" cfg.server.path}" ]
            ++ lib.mapAttrsToList (name: value: "${name}=${value}") cfg.server.environment;
          EnvironmentFile = cfg.server.environmentFiles;
        };
        Install.WantedBy = [ "default.target" ];
      };
    })
  ]);
}
