# run yarn tauri build

yarn tauri build
yarn tauri build --debug

# rename the builds, "bili-shadowreplay" to "bili-shadowreplay-cpu"
Get-ChildItem -Path ./src-tauri/target/release/bundle/msi/ | ForEach-Object {
    $newName = $_.Name -replace 'bili-shadowreplay', 'bili-shadowreplay-cpu'
    Rename-Item -Path $_.FullName -NewName $newName
}
Get-ChildItem -Path ./src-tauri/target/release/bundle/nsis/ | ForEach-Object {
    $newName = $_.Name -replace 'bili-shadowreplay', 'bili-shadowreplay-cpu'
    Rename-Item -Path $_.FullName -NewName $newName
}

# rename the debug builds, "bili-shadowreplay" to "bili-shadowreplay-cpu"
Get-ChildItem -Path ./src-tauri/target/debug/bundle/msi/ | ForEach-Object {
    $newName = $_.Name -replace 'bili-shadowreplay', 'bili-shadowreplay-cpu'
    Rename-Item -Path $_.FullName -NewName $newName
}
Get-ChildItem -Path ./src-tauri/target/debug/bundle/nsis/ | ForEach-Object {
    $newName = $_.Name -replace 'bili-shadowreplay', 'bili-shadowreplay-cpu'
    Rename-Item -Path $_.FullName -NewName $newName
}

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
