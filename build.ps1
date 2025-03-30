# run yarn tauri build

yarn tauri build
yarn tauri build --debug

# move the build to the correct location
Move-Item ./src-tauri/target/release/bundle/msi/* ./src-tauri/target/
Move-Item ./src-tauri/target/release/bundle/nsis/* ./src-tauri/target/

# rename debug builds to add "-debug" suffix
Get-ChildItem -Path ./src-tauri/target/debug/bundle/msi/ | ForEach-Object {
    $newName = $_.Name -replace '\.msi$', '-debug.msi'
    Rename-Item -Path $_.FullName -NewName $newName
}
Get-ChildItem -Path ./src-tauri/target/debug/bundle/nsis/ | ForEach-Object {
    $newName = $_.Name -replace '\.exe$', '-debug.exe'
    Rename-Item -Path $_.FullName -NewName $newName
}

# move the debug builds to the correct location
Move-Item ./src-tauri/target/debug/bundle/msi/* ./src-tauri/target/
Move-Item ./src-tauri/target/debug/bundle/nsis/* ./src-tauri/target/