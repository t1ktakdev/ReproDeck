param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class ReproDeckWindowCapture {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr handle, out RECT rect);
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr handle);
    [DllImport("user32.dll")]
    public static extern bool ShowWindowAsync(IntPtr handle, int command);
    [DllImport("user32.dll")]
    public static extern bool SetWindowPos(IntPtr handle, IntPtr insertAfter, int x, int y, int width, int height, uint flags);
}
"@

$process = Get-Process reprodeck -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $process) { throw "A visible ReproDeck window was not found." }
[ReproDeckWindowCapture]::ShowWindowAsync($process.MainWindowHandle, 9) | Out-Null
Start-Sleep -Milliseconds 120
$rect = New-Object ReproDeckWindowCapture+RECT
if (-not [ReproDeckWindowCapture]::GetWindowRect($process.MainWindowHandle, [ref]$rect)) {
    throw "Unable to read the ReproDeck window bounds."
}
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -lt 640 -or $height -lt 480) { throw "The ReproDeck window bounds are invalid." }

$absoluteOutput = [System.IO.Path]::GetFullPath($OutputPath)
$directory = Split-Path -Parent $absoluteOutput
New-Item -ItemType Directory -Force -Path $directory | Out-Null
[ReproDeckWindowCapture]::SetWindowPos($process.MainWindowHandle, [IntPtr](-1), 0, 0, 0, 0, 0x0003) | Out-Null
[ReproDeckWindowCapture]::SetWindowPos($process.MainWindowHandle, [IntPtr](-2), 0, 0, 0, 0, 0x0003) | Out-Null
[ReproDeckWindowCapture]::SetForegroundWindow($process.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 180
$bitmap = New-Object System.Drawing.Bitmap($width, $height)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $bitmap.Save($absoluteOutput, [System.Drawing.Imaging.ImageFormat]::Png)
}
finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}
Write-Output $absoluteOutput
