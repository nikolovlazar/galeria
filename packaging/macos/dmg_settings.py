# Settings for dmgbuild (https://dmgbuild.readthedocs.io): how the installer
# window looks when the disk image is opened. Run by packaging/macos/dmg.sh.
#
# The background is drawn by Galeria itself (src/ui/installer.rs). The window
# size and the icon positions below must match SIZE, ICON, APP_AT and
# APPLICATIONS_AT there; a test in that file checks they do.

app = defines["app"]  # noqa: F821  (dmgbuild provides `defines`)
background = defines["background"]  # noqa: F821

format = "UDZO"
compression_level = 9
files = [app]
symlinks = {"Applications": "/Applications"}

WIDTH, HEIGHT = 720, 460
ICON = 112
APP_AT = (190, 200)
APPLICATIONS_AT = (530, 200)

# Finder's title bar is part of the window's height.
window_rect = ((240, 160), (WIDTH, HEIGHT + 28))
default_view = "icon-view"
show_toolbar = False
show_status_bar = False
show_pathbar = False
show_sidebar = False
show_tab_view = False
show_icon_preview = False
include_icon_view_settings = True
include_list_view_settings = False

arrange_by = None
icon_size = ICON
text_size = 12
label_pos = "bottom"
icon_locations = {
    "Galeria.app": APP_AT,
    "Applications": APPLICATIONS_AT,
}
