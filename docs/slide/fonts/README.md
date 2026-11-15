# Murecho

The slides and editable draw.io figures use the official [Murecho variable font](https://github.com/google/fonts/blob/main/ofl/murecho/Murecho%5Bwght%5D.ttf) from Google Fonts. The bundled font is released under the license in [OFL.txt](OFL.txt).

All text cells in the `.drawio` files select Murecho as a Google Font with `fontFamily=Murecho` and a Google Fonts `fontSource`. The bundled font is also available for offline editing and export. On Windows, run [install_windows.ps1](install_windows.ps1) to install it for the current user before opening the diagrams in draw.io Desktop. Reopen draw.io after installation if it was already running.

On WSL2/Linux, copy `Murecho-wght.ttf` to `~/.local/share/fonts/` and run `fc-cache -f ~/.local/share/fonts/` to register it for the current user.

SVG exports embed a subset of Murecho so text displays correctly without a font installation or access to Google Fonts. After editing a draw.io label, regenerate the subset and all SVGs with `export_figures.sh`. That script needs the Python packages in `../requirements-fonts.txt`.

Install the bundled font before exporting the `.drawio` diagrams so the SVG and PDF use the requested typeface.
