#!/usr/bin/env bash
# Real desktop native acceptance. All profiles and logs remain in the printed directory.
set -euo pipefail
mkdir -p /tmp/ppl-ide-e2e
NATIVE_TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/prime-native-side-XXXXXX")"
NATIVE_TEST_HERE="$(cd "$(dirname "$0")" && pwd)"
export PPL_IDE_NO_DEVICE=1
# Discover the installed algebra helper before isolating application data.
if [[ -z "${PPL_CAS_HELPER:-}" ]]; then
  if [[ -n "${PPL_BIN:-}" && -x "$(dirname "$PPL_BIN")/ppl-giac-helper" ]]; then
    export PPL_CAS_HELPER="$(dirname "$PPL_BIN")/ppl-giac-helper"
  fi
  NATIVE_TEST_HELPER="${XDG_DATA_HOME:-$HOME/.local/share}/hp-prime-emulator/cas/ppl-giac-helper"
  if [[ -z "${PPL_CAS_HELPER:-}" && -x "$NATIVE_TEST_HELPER" ]]; then export PPL_CAS_HELPER="$NATIVE_TEST_HELPER"; fi
fi
# Keep default raster checks independent of a user's installed font selection.
unset PPL_PRIME_FONT_RESOURCE PPL_FACTORY_A PPL_FIRMWARE_INIT_DATA
export PPL_CONFIG=/dev/null
for scenario in input history history_reopen textbook presentation presentation_reopen editing editing_reopen formatted formatted_reopen state_create state_reopen graph dialogs user integer clipboard models solve graph_context special_views worksheet worksheet_reopen app_views app_views_reopen tools tools_reopen sin_pointer home_chrome palette utf16 utf16_reopen cas_special cas_special_reopen cas_arguments settings settings_reopen expr_catalog expr_catalog_reopen statistics statistics_reopen word_metadata word_metadata_reopen geometry geometry_reopen spreadsheet_grid spreadsheet_grid_reopen spreadsheet_forms spreadsheet_forms_reopen qualified qualified_reopen gallery gallery_reopen raster function_symbolic function_symbolic_reopen function_setup function_setup_reopen function_service function_service_reopen mouse function_numeric function_numeric_reopen function_numeric_exact function_numeric_exact_reopen function_numeric_controls function_definitions function_definitions_reopen current_numbers current_numbers_reopen; do
  profile="$scenario"
  if [[ "$scenario" == state_* ]]; then profile=state; fi
  if [[ "$scenario" == history* ]]; then profile=history; fi
  if [[ "$scenario" == presentation* ]]; then profile=presentation; fi
  if [[ "$scenario" == editing* ]]; then profile=editing; fi
  if [[ "$scenario" == formatted* ]]; then profile=formatted; fi
  if [[ "$scenario" == worksheet* ]]; then profile=worksheet; fi
  if [[ "$scenario" == app_views* ]]; then profile=app_views; fi
  if [[ "$scenario" == tools* ]]; then profile=tools; fi
  if [[ "$scenario" == settings* ]]; then profile=settings; fi
  if [[ "$scenario" == statistics* ]]; then profile=statistics; fi
  if [[ "$scenario" == word_metadata* ]]; then profile=word_metadata; fi
  if [[ "$scenario" == geometry* ]]; then profile=geometry; fi
  if [[ "$scenario" == spreadsheet_grid* ]]; then profile=spreadsheet_grid; fi
  if [[ "$scenario" == spreadsheet_forms* ]]; then profile=spreadsheet_forms; fi
  if [[ "$scenario" == qualified* ]]; then profile=qualified; fi
  if [[ "$scenario" == gallery* ]]; then profile=gallery; fi
  if [[ "$scenario" == function_symbolic* ]]; then profile=function_symbolic; fi
  if [[ "$scenario" == function_setup* ]]; then profile=function_setup; fi
  if [[ "$scenario" == function_service* ]]; then profile=function_service; fi
  if [[ "$scenario" == function_definitions* ]]; then profile=function_definitions; fi
  if [[ "$scenario" == current_numbers* ]]; then profile=current_numbers; fi
  if [[ "$scenario" == function_numeric* ]]; then profile="${scenario%_reopen}"; fi
  if [[ "$scenario" == utf16* ]]; then profile=utf16; fi
  if [[ "$scenario" == cas_special* ]]; then profile=cas_special; fi
  if [[ "$scenario" == expr_catalog* ]]; then profile=expr_catalog; fi
  export PPL_NATIVE_PROFILE="$NATIVE_TEST_ROOT/$profile/memory"
  export XDG_CONFIG_HOME="$NATIVE_TEST_ROOT/$profile/config"
  export XDG_DATA_HOME="$NATIVE_TEST_ROOT/$profile/data"
  export PPL_IDE_LOG_DIR="$NATIVE_TEST_ROOT/$scenario/log"
  native_scenario_timeout=60
  if [[ "$scenario" == qualified* ]]; then native_scenario_timeout=90; fi
  bash "$NATIVE_TEST_HERE/run_ide_auto.sh" "$NATIVE_TEST_HERE/scripts/script_native_side_$scenario.json" "$native_scenario_timeout"
done
# Optional real installed resources; no vendor font fixture is bundled.
if [[ -n "${PRIME_TEST_FONT_RESOURCE:-}" ]]; then
  export PPL_PRIME_FONT_RESOURCE="$PRIME_TEST_FONT_RESOURCE"
  export PPL_NATIVE_PROFILE="$NATIVE_TEST_ROOT/fonts/memory"
  export XDG_CONFIG_HOME="$NATIVE_TEST_ROOT/fonts/config"
  export XDG_DATA_HOME="$NATIVE_TEST_ROOT/fonts/data"
  export PPL_IDE_LOG_DIR="$NATIVE_TEST_ROOT/fonts/log"
  bash "$NATIVE_TEST_HERE/run_ide_auto.sh" "$NATIVE_TEST_HERE/scripts/script_native_side_fonts.json" 60
  python3 "$NATIVE_TEST_HERE/assert_native_fonts.py" /tmp/ppl-ide-e2e
  for scenario in menus text_units; do
    export PPL_NATIVE_PROFILE="$NATIVE_TEST_ROOT/fonts-$scenario/memory"
    export XDG_CONFIG_HOME="$NATIVE_TEST_ROOT/fonts-$scenario/config"
    export XDG_DATA_HOME="$NATIVE_TEST_ROOT/fonts-$scenario/data"
    export PPL_IDE_LOG_DIR="$NATIVE_TEST_ROOT/fonts-$scenario/log"
    bash "$NATIVE_TEST_HERE/run_ide_auto.sh" "$NATIVE_TEST_HERE/scripts/script_native_side_$scenario.json" 60
    python3 "$NATIVE_TEST_HERE/assert_native_$scenario.py" /tmp/ppl-ide-e2e
  done
  export PPL_NATIVE_PROFILE="$NATIVE_TEST_ROOT/fonts-settings/memory"
  export XDG_CONFIG_HOME="$NATIVE_TEST_ROOT/fonts-settings/config"
  export XDG_DATA_HOME="$NATIVE_TEST_ROOT/fonts-settings/data"
  for scenario in settings settings_reopen gallery gallery_reopen raster function_symbolic function_symbolic_reopen function_setup function_setup_reopen function_service function_service_reopen mouse function_numeric function_numeric_reopen function_numeric_exact function_numeric_exact_reopen function_numeric_controls function_definitions function_definitions_reopen current_numbers current_numbers_reopen; do
    native_font_profile="${scenario%_reopen}"
    export PPL_NATIVE_PROFILE="$NATIVE_TEST_ROOT/fonts-$native_font_profile/memory"
    export XDG_CONFIG_HOME="$NATIVE_TEST_ROOT/fonts-$native_font_profile/config"
    export XDG_DATA_HOME="$NATIVE_TEST_ROOT/fonts-$native_font_profile/data"
    export PPL_IDE_LOG_DIR="$NATIVE_TEST_ROOT/fonts-$scenario/log"
    bash "$NATIVE_TEST_HERE/run_ide_auto.sh" "$NATIVE_TEST_HERE/scripts/script_native_side_$scenario.json" 90
  done
  python3 "$NATIVE_TEST_HERE/assert_native_settings.py" /tmp/ppl-ide-e2e --prime
fi
python3 "$NATIVE_TEST_HERE/assert_native_side_input.py"
python3 "$NATIVE_TEST_HERE/assert_native_history.py"
python3 "$NATIVE_TEST_HERE/assert_native_textbook.py"
python3 "$NATIVE_TEST_HERE/assert_native_presentation.py"
python3 "$NATIVE_TEST_HERE/assert_native_editing.py"
python3 "$NATIVE_TEST_HERE/assert_native_formatted.py"
python3 "$NATIVE_TEST_HERE/assert_native_side_workflows.py"
python3 "$NATIVE_TEST_HERE/assert_native_special_views.py"
python3 "$NATIVE_TEST_HERE/assert_native_worksheet.py"
python3 "$NATIVE_TEST_HERE/assert_native_app_views.py"
python3 "$NATIVE_TEST_HERE/assert_native_tools.py"
python3 "$NATIVE_TEST_HERE/assert_native_sin_pointer.py"
python3 "$NATIVE_TEST_HERE/assert_native_home_chrome.py"
python3 "$NATIVE_TEST_HERE/assert_native_settings.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics.py"
python3 "$NATIVE_TEST_HERE/assert_native_word_metadata.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_grid.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_forms.py"
python3 "$NATIVE_TEST_HERE/assert_native_qualified.py"
python3 "$NATIVE_TEST_HERE/assert_native_gallery.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_symbolic.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_setup.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_service.py"
python3 "$NATIVE_TEST_HERE/assert_native_mouse.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_numeric.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_numeric_exact.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_numeric_controls.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_definitions.py"
python3 "$NATIVE_TEST_HERE/assert_native_current_numbers.py"
python3 "$NATIVE_TEST_HERE/assert_native_raster.py"
python3 "$NATIVE_TEST_HERE/assert_native_palette.py"
python3 "$NATIVE_TEST_HERE/assert_native_utf16.py"
python3 "$NATIVE_TEST_HERE/assert_native_cas_special.py"
python3 "$NATIVE_TEST_HERE/assert_native_cas_arguments.py"
python3 "$NATIVE_TEST_HERE/assert_native_expr_catalog.py"
printf 'Preserved Native desktop evidence: %s\n' "$NATIVE_TEST_ROOT"
