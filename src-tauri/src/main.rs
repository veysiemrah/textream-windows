// Release builds attach no console: a teleprompter launched from Explorer must
// not flash a terminal window behind the presenter mid-stream.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    textream_lib::run()
}
