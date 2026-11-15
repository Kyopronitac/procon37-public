$ErrorActionPreference = 'Stop'

$source = Join-Path $PSScriptRoot 'Murecho-wght.ttf'
$fontDirectory = Join-Path $env:LOCALAPPDATA 'Microsoft\Windows\Fonts'
$destination = Join-Path $fontDirectory 'Murecho-wght.ttf'
$registryPath = 'HKCU:\Software\Microsoft\Windows NT\CurrentVersion\Fonts'

New-Item -ItemType Directory -Path $fontDirectory -Force | Out-Null
Copy-Item -LiteralPath $source -Destination $destination -Force
New-Item -Path $registryPath -Force | Out-Null
New-ItemProperty -Path $registryPath -Name 'Murecho (TrueType)' `
  -PropertyType String -Value $destination -Force | Out-Null

Add-Type @'
using System;
using System.Runtime.InteropServices;

public static class FontRegistration {
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)]
    public static extern int AddFontResourceEx(string file, uint flags, IntPtr reserved);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr SendMessageTimeout(IntPtr window, uint message,
        IntPtr wParam, IntPtr lParam, uint flags, uint timeout, out IntPtr result);
}
'@

$added = [FontRegistration]::AddFontResourceEx($destination, 0, [IntPtr]::Zero)
$result = [IntPtr]::Zero
[FontRegistration]::SendMessageTimeout([IntPtr]0xffff, 0x001d,
  [IntPtr]::Zero, [IntPtr]::Zero, 0x0002, 5000, [ref]$result) | Out-Null

Write-Output "Installed Murecho for the current Windows user: $destination"
Write-Output "Fonts added to this session: $added"
