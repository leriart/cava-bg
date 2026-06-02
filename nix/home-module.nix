{ config, lib, pkgs, ... }:

let
  cfg = config.programs.cava-bg;
  inherit (lib)
    types mkOption mkEnableOption mkIf mkMerge literalExpression;
  format = pkgs.formats.toml { };
  configFile = format.generate "cava-bg-config.toml" cfg.settings;
in
{
  options.programs.cava-bg = {
    enable = mkEnableOption "cava-bg, the X-Ray wallpaper engine for Wayland";

    package = mkOption {
      type = types.nullOr types.package;
      default = null;
      example = literalExpression "inputs.cava-bg.packages.${pkgs.system}.default";
      description = ''
        The cava-bg package to use.

        This option is automatically populated when the module is imported
        from the cava-bg flake via `homeManagerModules.cava-bg`.
        Set manually only if you need to override the default package.
      '';
    };

    settings = mkOption {
      type = format.type;
      default = { };
      example = literalExpression ''
        {
          general.framerate = 60;
          audio.bar_count = 76;
        }
      '';
      description = ''
        Declarative configuration for cava-bg, mirroring the structure of `config.toml`.
      '';
    };

    systemd = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Whether to manage the cava-bg daemon via a systemd user service.
        '';
      };

      memoryMax = mkOption {
        type = types.nullOr types.str;
        default = "500M";
        example = "1G";
        description = ''
          Hard memory limit for the cava-bg daemon (systemd `MemoryMax`).
          Set to `null` or `"infinity"` to disable.
        '';
      };
    };
  };

  config = mkIf cfg.enable (mkMerge [
    {
      assertions = [
        {
          assertion = cfg.package != null;
          message = ''
            programs.cava-bg.package must be set when enable = true.
            This is normally handled automatically when importing the module
            from the cava-bg flake via `inputs.cava-bg.homeManagerModules.cava-bg`.
          '';
        }
      ];
    }

    {
      home.packages = lib.optional (cfg.package != null) cfg.package;

      xdg.configFile."cava-bg/config.toml" = mkIf (cfg.settings != { }) {
        source = configFile;
      };

      systemd.user.services.cava-bg = mkIf cfg.systemd.enable {
        Unit = {
          Description = "Cava-BG Visualizer Daemon";
          PartOf = [ "graphical-session.target" ];
          After = [ "graphical-session.target" ];
        };

        Service = {
          ExecStart = "${cfg.package}/bin/cava-bg on --debug";

          ExecStop = "${cfg.package}/bin/cava-bg off";

          Restart = "on-failure";
          RestartSec = 5;
        } // lib.optionalAttrs (cfg.systemd.memoryMax != null) {
          MemoryMax = cfg.systemd.memoryMax;
        };

        Install = {
          WantedBy = [ "graphical-session.target" ];
        };
      };
    }
  ]);
}
