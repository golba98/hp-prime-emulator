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
for scenario in input history history_reopen textbook textbook_radicals textbook_cas_matrix textbook_delimiters textbook_calls textbook_calls_reopen textbook_lists textbook_lists_reopen presentation presentation_reopen editing editing_reopen formatted formatted_reopen state_create state_reopen graph dialogs user integer clipboard models solve graph_context special_views worksheet worksheet_reopen app_views app_views_reopen tools tools_reopen note_bullets note_bullets_reopen sin_pointer home_chrome palette utf16 utf16_reopen cas_special cas_special_reopen cas_history_simplify cas_history_simplify_reopen cas_arguments settings settings_reopen expr_catalog expr_catalog_reopen statistics statistics_reopen statistics_one_charts statistics_one_charts_reopen statistics_one_scale statistics_one_scale_reopen statistics_one_reset statistics_one_reset_reopen statistics_two_scale statistics_two_scale_reopen statistics_check statistics_check_reopen statistics_two_fit statistics_two_fit_reopen statistics_two_writes statistics_two_writes_reopen statistics_two_reset statistics_two_reset_reopen statistics_two_cache statistics_two_cache_reopen statistics_one_trace statistics_one_definition statistics_one_definition_reopen statistics_geometry statistics_geometry_reopen statistics_fractional statistics_fractional_reopen statistics_unit statistics_unit_reopen statistics_infinite_autoscale statistics_infinite_autoscale_reopen word_metadata word_metadata_reopen geometry geometry_reopen geometry_bridge geometry_bridge_reopen geometry_settings geometry_settings_reopen geometry_cross_app geometry_cross_app_reopen geometry_curve_point geometry_curve_point_reopen geometry_tangent geometry_tangent_reopen geometry_function geometry_function_reopen geometry_trace geometry_trace_reopen geometry_dependent_trace geometry_polygons geometry_parabola geometry_parabola_reopen geometry_hyperbola geometry_hyperbola_reopen geometry_conic geometry_conic_reopen geometry_locus geometry_locus_reopen geometry_maps geometry_maps_reopen geometry_plane geometry_plane_reopen geometry_field_wizard geometry_field_wizard_reopen geometry_slider geometry_slider_reopen geometry_scroll_text geometry_scroll_text_reopen geometry_ellipse geometry_ellipse_reopen geometry_arc geometry_arc_reopen geometry_curve_intersection geometry_curve_intersection_reopen geometry_reset geometry_reset_reopen spreadsheet_grid spreadsheet_grid_reopen spreadsheet_forms spreadsheet_forms_reopen spreadsheet_attributes spreadsheet_attributes_reopen spreadsheet_format_palette spreadsheet_format_palette_reopen spreadsheet_headers spreadsheet_headers_reopen spreadsheet_clipboard spreadsheet_clipboard_reopen spreadsheet_stat1 spreadsheet_stat1_reopen spreadsheet_inference spreadsheet_inference_reopen spreadsheet_stat2 spreadsheet_stat2_reopen spreadsheet_regrs spreadsheet_regrs_reopen spreadsheet_amort spreadsheet_amort_reopen spreadsheet_sort spreadsheet_sort_reopen spreadsheet_typed_sort spreadsheet_typed_sort_reopen spreadsheet_choose spreadsheet_choose_reopen spreadsheet_external_clipboard spreadsheet_external_clipboard_reopen spreadsheet_prediction spreadsheet_prediction_reopen spreadsheet_predx_polynomial spreadsheet_predx_polynomial_reopen spreadsheet_home_history spreadsheet_home_history_reopen spreadsheet_cas_history spreadsheet_cas_history_reopen spreadsheet_cas_toggle spreadsheet_cas_toggle_reopen qualified qualified_reopen gallery gallery_reopen graph3d graph3d_reopen graph3d_setup graph3d_setup_reopen graph3d_numeric graph3d_numeric_reopen graph3d_numeric_navigation graph3d_numeric_navigation_reopen graph3d_numeric_custom graph3d_numeric_custom_reopen graph3d_plot_goto graph3d_plot_goto_reopen graph3d_plot_definition graph3d_plot_definition_reopen graph3d_colors graph3d_colors_reopen graph3d_illumination graph3d_plot_zoom graph3d_plot_zoom_reopen raster triangle_raster invert_raster polygon_raster stddev_overloads stddev_signed stddev_signed_vector stddev_signed_matrix function_symbolic function_symbolic_reopen function_checkbox function_checkbox_reopen function_setup function_setup_reopen function_service function_service_reopen mouse function_numeric function_numeric_reopen function_numeric_exact function_numeric_exact_reopen function_numeric_controls function_definitions function_definitions_reopen current_numbers current_numbers_reopen; do
  profile="$scenario"
  if [[ "$scenario" == textbook_calls* ]]; then profile=textbook_calls; fi
  if [[ "$scenario" == textbook_lists* ]]; then profile=textbook_lists; fi
  if [[ "$scenario" == state_* ]]; then profile=state; fi
  if [[ "$scenario" == history* ]]; then profile=history; fi
  if [[ "$scenario" == presentation* ]]; then profile=presentation; fi
  if [[ "$scenario" == editing* ]]; then profile=editing; fi
  if [[ "$scenario" == formatted* ]]; then profile=formatted; fi
  if [[ "$scenario" == worksheet* ]]; then profile=worksheet; fi
  if [[ "$scenario" == app_views* ]]; then profile=app_views; fi
  if [[ "$scenario" == tools* ]]; then profile=tools; fi
  if [[ "$scenario" == note_bullets* ]]; then profile=note_bullets; fi
  if [[ "$scenario" == settings* ]]; then profile=settings; fi
  if [[ "$scenario" == statistics* ]]; then profile=statistics; fi
  if [[ "$scenario" == statistics_one_charts* ]]; then profile=statistics_one_charts; fi
  if [[ "$scenario" == statistics_one_scale* ]]; then profile=statistics_one_scale; fi
  if [[ "$scenario" == statistics_one_reset* ]]; then profile=statistics_one_reset; fi
  if [[ "$scenario" == statistics_two_scale* ]]; then profile=statistics_two_scale; fi
  if [[ "$scenario" == statistics_check* ]]; then profile=statistics_check; fi
  if [[ "$scenario" == statistics_two_fit* ]]; then profile=statistics_two_fit; fi
  if [[ "$scenario" == statistics_two_writes* ]]; then profile=statistics_two_writes; fi
  if [[ "$scenario" == statistics_two_reset* ]]; then profile=statistics_two_reset; fi
  if [[ "$scenario" == statistics_two_cache* ]]; then profile=statistics_two_cache; fi
  if [[ "$scenario" == statistics_one_trace ]]; then profile=statistics_one_trace; fi
  if [[ "$scenario" == statistics_one_definition* ]]; then profile=statistics_one_definition; fi
  if [[ "$scenario" == statistics_geometry* ]]; then profile=statistics_geometry; fi
  if [[ "$scenario" == statistics_fractional* ]]; then profile=statistics_fractional; fi
  if [[ "$scenario" == statistics_unit* ]]; then profile=statistics_unit; fi
  if [[ "$scenario" == statistics_infinite_autoscale* ]]; then profile=statistics_infinite_autoscale; fi
  if [[ "$scenario" == word_metadata* ]]; then profile=word_metadata; fi
  if [[ "$scenario" == geometry* ]]; then profile=geometry; fi
  if [[ "$scenario" == geometry_bridge* ]]; then profile=geometry_bridge; fi
  if [[ "$scenario" == geometry_settings* ]]; then profile=geometry_settings; fi
  if [[ "$scenario" == geometry_cross_app* ]]; then profile=geometry_cross_app; fi
  if [[ "$scenario" == geometry_curve_point* ]]; then profile=geometry_curve_point; fi
  if [[ "$scenario" == geometry_tangent* ]]; then profile=geometry_tangent; fi
  if [[ "$scenario" == geometry_function* ]]; then profile=geometry_function; fi
  if [[ "$scenario" == geometry_slider* ]]; then profile=geometry_slider; fi
  if [[ "$scenario" == geometry_field_wizard* ]]; then profile=geometry_field_wizard; fi
  if [[ "$scenario" == geometry_plane* ]]; then profile=geometry_plane; fi
  if [[ "$scenario" == geometry_maps* ]]; then profile=geometry_maps; fi
  if [[ "$scenario" == geometry_locus* ]]; then profile=geometry_locus; fi
  if [[ "$scenario" == geometry_conic* ]]; then profile=geometry_conic; fi
  if [[ "$scenario" == geometry_hyperbola* ]]; then profile=geometry_hyperbola; fi
  if [[ "$scenario" == geometry_parabola* ]]; then profile=geometry_parabola; fi
  if [[ "$scenario" == geometry_polygons ]]; then profile=geometry_polygons; fi
  if [[ "$scenario" == geometry_dependent_trace ]]; then profile=geometry_dependent_trace; fi
  if [[ "$scenario" == geometry_trace* ]]; then profile=geometry_trace; fi
  if [[ "$scenario" == geometry_scroll_text* ]]; then profile=geometry_scroll_text; fi
  if [[ "$scenario" == geometry_ellipse* ]]; then profile=geometry_ellipse; fi
  if [[ "$scenario" == geometry_arc* ]]; then profile=geometry_arc; fi
  if [[ "$scenario" == geometry_curve_intersection* ]]; then profile=geometry_curve_intersection; fi
  if [[ "$scenario" == geometry_reset* ]]; then profile=geometry_reset; fi
  if [[ "$scenario" == spreadsheet_grid* ]]; then profile=spreadsheet_grid; fi
  if [[ "$scenario" == spreadsheet_forms* ]]; then profile=spreadsheet_forms; fi
  if [[ "$scenario" == spreadsheet_attributes* ]]; then profile=spreadsheet_attributes; fi
  if [[ "$scenario" == spreadsheet_format_palette* ]]; then profile=spreadsheet_format_palette; fi
  if [[ "$scenario" == spreadsheet_headers* ]]; then profile=spreadsheet_headers; fi
  if [[ "$scenario" == spreadsheet_clipboard* ]]; then profile=spreadsheet_clipboard; fi
  if [[ "$scenario" == spreadsheet_amort* ]]; then profile=spreadsheet_amort; fi
  if [[ "$scenario" == spreadsheet_regrs* ]]; then profile=spreadsheet_regrs; fi
  if [[ "$scenario" == spreadsheet_stat2* ]]; then profile=spreadsheet_stat2; fi
  if [[ "$scenario" == spreadsheet_inference* ]]; then profile=spreadsheet_inference; fi
  if [[ "$scenario" == spreadsheet_stat1* ]]; then profile=spreadsheet_stat1; fi
  if [[ "$scenario" == spreadsheet_sort* ]]; then profile=spreadsheet_sort; fi
  if [[ "$scenario" == spreadsheet_typed_sort* ]]; then profile=spreadsheet_typed_sort; fi
  if [[ "$scenario" == spreadsheet_choose* ]]; then profile=spreadsheet_choose; fi
  if [[ "$scenario" == spreadsheet_external_clipboard* ]]; then profile=spreadsheet_external_clipboard; fi
  if [[ "$scenario" == spreadsheet_prediction* ]]; then profile=spreadsheet_prediction; fi
  if [[ "$scenario" == spreadsheet_predx_polynomial* ]]; then profile=spreadsheet_predx_polynomial; fi
  if [[ "$scenario" == spreadsheet_home_history* ]]; then profile=spreadsheet_home_history; fi
  if [[ "$scenario" == spreadsheet_cas_history* ]]; then profile=spreadsheet_cas_history; fi
  if [[ "$scenario" == spreadsheet_cas_toggle* ]]; then profile=spreadsheet_cas_toggle; fi
  if [[ "$scenario" == qualified* ]]; then profile=qualified; fi
  if [[ "$scenario" == gallery* ]]; then profile=gallery; fi
  if [[ "$scenario" == graph3d* ]]; then profile=graph3d; fi
  if [[ "$scenario" == graph3d_setup* ]]; then profile=graph3d_setup; fi
  if [[ "$scenario" == graph3d_plot_zoom* ]]; then profile=graph3d_plot_zoom;
  elif [[ "$scenario" == graph3d_colors* ]]; then profile=graph3d_colors;
  elif [[ "$scenario" == graph3d_plot_definition* ]]; then profile=graph3d_plot_definition;
  elif [[ "$scenario" == graph3d_plot_goto* ]]; then profile=graph3d_plot_goto;
  elif [[ "$scenario" == graph3d_numeric_navigation* ]]; then profile=graph3d_numeric_navigation;
  elif [[ "$scenario" == graph3d_numeric_custom* ]]; then profile=graph3d_numeric_custom;
  elif [[ "$scenario" == graph3d_numeric* ]]; then profile=graph3d_numeric; fi
  if [[ "$scenario" == function_symbolic* ]]; then profile=function_symbolic; fi
  if [[ "$scenario" == function_checkbox* ]]; then profile=function_checkbox; fi
  if [[ "$scenario" == function_setup* ]]; then profile=function_setup; fi
  if [[ "$scenario" == function_service* ]]; then profile=function_service; fi
  if [[ "$scenario" == function_definitions* ]]; then profile=function_definitions; fi
  if [[ "$scenario" == current_numbers* ]]; then profile=current_numbers; fi
  if [[ "$scenario" == function_numeric* ]]; then profile="${scenario%_reopen}"; fi
  if [[ "$scenario" == utf16* ]]; then profile=utf16; fi
  if [[ "$scenario" == cas_special* ]]; then profile=cas_special; fi
  if [[ "$scenario" == cas_history_simplify* ]]; then profile=cas_history_simplify; fi
  if [[ "$scenario" == expr_catalog* ]]; then profile=expr_catalog; fi
  export PPL_NATIVE_PROFILE="$NATIVE_TEST_ROOT/$profile/memory"
  export XDG_CONFIG_HOME="$NATIVE_TEST_ROOT/$profile/config"
  export XDG_DATA_HOME="$NATIVE_TEST_ROOT/$profile/data"
  export PPL_IDE_LOG_DIR="$NATIVE_TEST_ROOT/$scenario/log"
  native_scenario_timeout=60
  if [[ "$scenario" == statistics ]]; then native_scenario_timeout=120; fi
  if [[ "$scenario" == qualified* ]]; then native_scenario_timeout=90; fi
  if [[ "$scenario" == statistics_one_trace ]]; then native_scenario_timeout=120; fi
  if [[ "$scenario" == statistics_one_definition ]]; then native_scenario_timeout=120; fi
  if [[ "$scenario" == spreadsheet_attributes ]]; then native_scenario_timeout=180; fi
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
# The installed Help archive is optional and stays outside the test fixtures.
if [[ -n "${PRIME_TEST_HELP_RESOURCE:-}" ]]; then
  unset PPL_PRIME_FONT_RESOURCE
  export PPL_PRIME_HELP_RESOURCE="$PRIME_TEST_HELP_RESOURCE"
  export PPL_NATIVE_PROFILE="$NATIVE_TEST_ROOT/help-installed/memory"
  export XDG_CONFIG_HOME="$NATIVE_TEST_ROOT/help-installed/config"
  export XDG_DATA_HOME="$NATIVE_TEST_ROOT/help-installed/data"
  for scenario in help_installed help_installed_reopen help_list help_list_reopen; do
    if [[ "$scenario" == help_list* ]]; then
      export PPL_NATIVE_PROFILE="$NATIVE_TEST_ROOT/help-lists/memory"
      export XDG_CONFIG_HOME="$NATIVE_TEST_ROOT/help-lists/config"
      export XDG_DATA_HOME="$NATIVE_TEST_ROOT/help-lists/data"
    fi
    export PPL_IDE_LOG_DIR="$NATIVE_TEST_ROOT/$scenario/log"
    bash "$NATIVE_TEST_HERE/run_ide_auto.sh" "$NATIVE_TEST_HERE/scripts/script_native_side_$scenario.json" 60
  done
  python3 "$NATIVE_TEST_HERE/assert_native_help.py" /tmp/ppl-ide-e2e
  python3 "$NATIVE_TEST_HERE/assert_native_help_list.py" /tmp/ppl-ide-e2e
fi
python3 "$NATIVE_TEST_HERE/assert_native_side_input.py"
python3 "$NATIVE_TEST_HERE/assert_native_history.py"
python3 "$NATIVE_TEST_HERE/assert_native_textbook.py"
python3 "$NATIVE_TEST_HERE/assert_native_textbook_radicals.py"
python3 "$NATIVE_TEST_HERE/assert_native_textbook_cas_matrix.py"
python3 "$NATIVE_TEST_HERE/assert_native_textbook_delimiters.py"
python3 "$NATIVE_TEST_HERE/assert_native_textbook_calls.py"
python3 "$NATIVE_TEST_HERE/assert_native_textbook_lists.py"
python3 "$NATIVE_TEST_HERE/assert_native_presentation.py"
python3 "$NATIVE_TEST_HERE/assert_native_editing.py"
python3 "$NATIVE_TEST_HERE/assert_native_formatted.py"
python3 "$NATIVE_TEST_HERE/assert_native_side_workflows.py"
python3 "$NATIVE_TEST_HERE/assert_native_special_views.py"
python3 "$NATIVE_TEST_HERE/assert_native_worksheet.py"
python3 "$NATIVE_TEST_HERE/assert_native_app_views.py"
python3 "$NATIVE_TEST_HERE/assert_native_tools.py"
python3 "$NATIVE_TEST_HERE/assert_native_note_bullets.py"
python3 "$NATIVE_TEST_HERE/assert_native_sin_pointer.py"
python3 "$NATIVE_TEST_HERE/assert_native_home_chrome.py"
python3 "$NATIVE_TEST_HERE/assert_native_settings.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_one_charts.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_one_scale.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_one_reset.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_two_scale.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_check.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_two_fit.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_two_writes.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_two_reset.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_two_cache.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_one_trace.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_one_definition.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_geometry.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_fractional.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_unit.py"
python3 "$NATIVE_TEST_HERE/assert_native_statistics_infinite_autoscale.py"
python3 "$NATIVE_TEST_HERE/assert_native_word_metadata.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_bridge.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_settings.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_cross_app.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_curve_point.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_tangent.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_function.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_trace.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_dependent_trace.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_polygons.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_parabola.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_hyperbola.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_conic.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_locus.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_maps.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_plane.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_field_wizard.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_slider.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_scroll_text.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_ellipse.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_arc.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_curve_intersection.py"
python3 "$NATIVE_TEST_HERE/assert_native_geometry_reset.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_grid.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_forms.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_attributes.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_format_palette.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_headers.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_clipboard.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_sort.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_inference.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_stat1.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_stat2.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_regrs.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_amort.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_typed_sort.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_choose.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_external_clipboard.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_prediction.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_predx_polynomial.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_home_history.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_cas_history.py"
python3 "$NATIVE_TEST_HERE/assert_native_spreadsheet_cas_toggle.py"
python3 "$NATIVE_TEST_HERE/assert_native_qualified.py"
python3 "$NATIVE_TEST_HERE/assert_native_gallery.py"
python3 "$NATIVE_TEST_HERE/assert_native_graph3d.py"
python3 "$NATIVE_TEST_HERE/assert_native_graph3d_plot_zoom.py"
python3 "$NATIVE_TEST_HERE/assert_native_graph3d_colors.py"
python3 "$NATIVE_TEST_HERE/assert_native_graph3d_illumination.py"
python3 "$NATIVE_TEST_HERE/assert_native_graph3d_plot_definition.py"
python3 "$NATIVE_TEST_HERE/assert_native_graph3d_plot_goto.py"
python3 "$NATIVE_TEST_HERE/assert_native_graph3d_numeric_custom.py"
python3 "$NATIVE_TEST_HERE/assert_native_graph3d_numeric_navigation.py"
python3 "$NATIVE_TEST_HERE/assert_native_graph3d_numeric.py"
python3 "$NATIVE_TEST_HERE/assert_native_graph3d_setup.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_symbolic.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_checkbox.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_setup.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_service.py"
python3 "$NATIVE_TEST_HERE/assert_native_mouse.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_numeric.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_numeric_exact.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_numeric_controls.py"
python3 "$NATIVE_TEST_HERE/assert_native_function_definitions.py"
python3 "$NATIVE_TEST_HERE/assert_native_current_numbers.py"
python3 "$NATIVE_TEST_HERE/assert_native_raster.py"
python3 "$NATIVE_TEST_HERE/assert_native_triangle_raster.py"
python3 "$NATIVE_TEST_HERE/assert_native_invert_raster.py"
python3 "$NATIVE_TEST_HERE/assert_native_polygon_raster.py"
python3 "$NATIVE_TEST_HERE/assert_native_stddev_overloads.py"
python3 "$NATIVE_TEST_HERE/assert_native_stddev_signed.py"
python3 "$NATIVE_TEST_HERE/assert_native_stddev_signed_vector.py"
python3 "$NATIVE_TEST_HERE/assert_native_stddev_signed_vector.py" /tmp/ppl-ide-e2e matrix
python3 "$NATIVE_TEST_HERE/assert_native_palette.py"
python3 "$NATIVE_TEST_HERE/assert_native_utf16.py"
python3 "$NATIVE_TEST_HERE/assert_native_cas_special.py"
python3 "$NATIVE_TEST_HERE/assert_native_cas_history_simplify.py"
python3 "$NATIVE_TEST_HERE/assert_native_cas_arguments.py"
python3 "$NATIVE_TEST_HERE/assert_native_expr_catalog.py"
printf 'Preserved Native desktop evidence: %s\n' "$NATIVE_TEST_ROOT"
