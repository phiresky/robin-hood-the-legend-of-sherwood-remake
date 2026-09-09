# Lutris — native Desura installer and language selection

- Original source: [Lutris — native Desura installer and language selection](https://lutris.net/games/install/7091/view)
- Author / publication: Lutris installer contributors
- Language / date: English; exact revision date not verified
- Access: Installer source and maintainer notes inspected; script not executed
- Checked: 2026-09-09
- Format: original summary and research notes; not a transcript.

The installer requests an existing Linux installer from Desura and launches data/robin. Its language-selection workaround reflects the maintainer's description of directory precedence: German data/1031, then French data/1036, with English data/2047 as fallback. The script renames unwanted language directories.

The notes recommend disabling the Lutris Runtime for the game to select its resolution correctly. This is historical maintainer guidance, not a verified requirement for current Lutris releases.

The description calls Desura the only available native edition, but that should not erase earlier CD and PowerPC releases documented elsewhere. No files were renamed or software installed during this research.

## Converted text from the original HTML


### technical__lutris-desura-script.html

_Source: `originals/technical__lutris-desura-script.html`._

[ Lutris ](/)

  * [Download](/downloads)
  * [Games](/games)
  * [About](/about)
  * [Discord](https://discordapp.com/invite/Pnt5CuY)
  * [Forums](https://forums.lutris.net)
  * [Donate](/donate)



[Log in](/user/login?next=/games/install/7091/view) [Register](/user/register)

# Installer robin-hood-the-legend-of-sher-desura

  * YAML (script only)
  * YAML
  * JSON


    
    
    files:
    - game: N/A:Select the Linux installer downloaded from Desura
    game:
      exe: data/robin
    installer:
    - input_menu:
        description: 'Choose the game''s language:'
        id: LANG
        options:
        - en: English
        - de: German
        - fr: French
        preselect: en
    - extract:
        description: Installing game data
        file: $game
        format: zip
    - extract:
        description: Installing binaries and executables
        dst: $GAMEDIR/data
        file: $GAMEDIR/data/bins.and.libs_LINUX_X86_32.tar.xz
    - execute:
        command: if [ "$INPUT_LANG" = "de" ]; then mv $GAMEDIR/data/1036 $GAMEDIR/data/disabled_1036
          && mv $GAMEDIR/data/2047 $GAMEDIR/data/disabled_2047; elif [ "$INPUT_LANG" =
          "fr" ]; then mv $GAMEDIR/data/1031 $GAMEDIR/data/disabled_1031 && mv $GAMEDIR/data/2047
          $GAMEDIR/data/disabled_2047; else mv $GAMEDIR/data/1031 $GAMEDIR/data/disabled_1031
          && mv $GAMEDIR/data/1036 $GAMEDIR/data/disabled_1036; fi
        description: Setting language
        env:
          GAMEDIR: $GAMEDIR
          INPUT_LANG: $INPUT_LANG
    
    
    
    description: The Desura version is the only native Linux version available.
    game_slug: robin-hood-the-legend-of-sherwood
    gogslug: robin_hood
    humblestoreid: ''
    installer_slug: robin-hood-the-legend-of-sher-desura
    name: 'Robin Hood: The Legend of Sherwood'
    notes: "* After installation, you should disable the Lutris Runtime in order to have\
      \ the game set the proper resolution by itself. Right click on the game -> \"Configure\"\
      \ -> Tab \"System options\" -> Check [x] \"Show advanced options\" (bottom left)\
      \ -> Check [x] \"Disable Lutris Runtime\".\r\n\r\n* The language cannot be set directly,\
      \ so there's a workaround in this installer - The game looks for the folder 'data/1031',\
      \ and if it exists, the language is German. If not, it looks for the folder 'data/1036',\
      \ and if that exists, it sets the language to French. The fallback 'data/2047' is\
      \ English."
    runner: linux
    script:
      files:
      - game: N/A:Select the Linux installer downloaded from Desura
      game:
        exe: data/robin
      installer:
      - input_menu:
          description: 'Choose the game''s language:'
          id: LANG
          options:
          - en: English
          - de: German
          - fr: French
          preselect: en
      - extract:
          description: Installing game data
          file: $game
          format: zip
      - extract:
          description: Installing binaries and executables
          dst: $GAMEDIR/data
          file: $GAMEDIR/data/bins.and.libs_LINUX_X86_32.tar.xz
      - execute:
          command: if [ "$INPUT_LANG" = "de" ]; then mv $GAMEDIR/data/1036 $GAMEDIR/data/disabled_1036
            && mv $GAMEDIR/data/2047 $GAMEDIR/data/disabled_2047; elif [ "$INPUT_LANG"
            = "fr" ]; then mv $GAMEDIR/data/1031 $GAMEDIR/data/disabled_1031 && mv $GAMEDIR/data/2047
            $GAMEDIR/data/disabled_2047; else mv $GAMEDIR/data/1031 $GAMEDIR/data/disabled_1031
            && mv $GAMEDIR/data/1036 $GAMEDIR/data/disabled_1036; fi
          description: Setting language
          env:
            GAMEDIR: $GAMEDIR
            INPUT_LANG: $INPUT_LANG
    slug: robin-hood-the-legend-of-sher-desura
    steamid: 46560
    version: Desura
    year: 2002
    
    
    
    {
      "game_slug": "robin-hood-the-legend-of-sherwood",
      "version": "Desura",
      "description": "The Desura version is the only native Linux version available.",
      "notes": "* After installation, you should disable the Lutris Runtime in order to have the game set the proper resolution by itself. Right click on the game -> \"Configure\" -> Tab \"System options\" -> Check [x] \"Show advanced options\" (bottom left) -> Check [x] \"Disable Lutris Runtime\".\r\n\r\n* The language cannot be set directly, so there's a workaround in this installer - The game looks for the folder 'data/1031', and if it exists, the language is German. If not, it looks for the folder 'data/1036', and if that exists, it sets the language to French. The fallback 'data/2047' is English.",
      "name": "Robin Hood: The Legend of Sherwood",
      "year": 2002,
      "steamid": 46560,
      "gogslug": "robin_hood",
      "humblestoreid": "",
      "runner": "linux",
      "slug": "robin-hood-the-legend-of-sher-desura",
      "installer_slug": "robin-hood-the-legend-of-sher-desura",
      "script": {
        "files": [
          {
            "game": "N/A:Select the Linux installer downloaded from Desura"
          }
        ],
        "game": {
          "exe": "data/robin"
        },
        "installer": [
          {
            "input_menu": {
              "description": "Choose the game's language:",
              "id": "LANG",
              "options": [
                {
                  "en": "English"
                },
                {
                  "de": "German"
                },
                {
                  "fr": "French"
                }
              ],
              "preselect": "en"
            }
          },
          {
            "extract": {
              "description": "Installing game data",
              "file": "$game",
              "format": "zip"
            }
          },
          {
            "extract": {
              "description": "Installing binaries and executables",
              "dst": "$GAMEDIR/data",
              "file": "$GAMEDIR/data/bins.and.libs_LINUX_X86_32.tar.xz"
            }
          },
          {
            "execute": {
              "command": "if [ \"$INPUT_LANG\" = \"de\" ]; then mv $GAMEDIR/data/1036 $GAMEDIR/data/disabled_1036 && mv $GAMEDIR/data/2047 $GAMEDIR/data/disabled_2047; elif [ \"$INPUT_LANG\" = \"fr\" ]; then mv $GAMEDIR/data/1031 $GAMEDIR/data/disabled_1031 && mv $GAMEDIR/data/2047 $GAMEDIR/data/disabled_2047; else mv $GAMEDIR/data/1031 $GAMEDIR/data/disabled_1031 && mv $GAMEDIR/data/1036 $GAMEDIR/data/disabled_1036; fi",
              "description": "Setting language",
              "env": {
                "GAMEDIR": "$GAMEDIR",
                "INPUT_LANG": "$INPUT_LANG"
              }
            }
          }
        ]
      }
    }

[Back to game](/games/robin-hood-the-legend-of-sherwood/)

[ __](https://github.com/lutris)

[ __](https://fosstodon.org/@lutris)

[ __](https://discordapp.com/invite/Pnt5CuY)

[ ](https://patreon.com/lutris)
