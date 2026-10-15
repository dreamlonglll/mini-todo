# 把图片按 JPEG 重新编码后以 base64 分段打印到日志（产物下载不可用时，从 Actions 日志还原截图用）。
# 格式：===IMG-BEGIN <名字>=== / 每行 1000 个 base64 字符 / ===IMG-END <名字>===
param(
    [Parameter(Mandatory = $true)][string]$Path,
    [string]$Name = [System.IO.Path]::GetFileNameWithoutExtension($Path),
    [int]$Quality = 80
)
Add-Type -AssemblyName System.Drawing
$img = [System.Drawing.Image]::FromFile((Resolve-Path $Path))
$codec = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq 'image/jpeg' }
$params = New-Object System.Drawing.Imaging.EncoderParameters 1
$params.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality), ([long]$Quality)
$ms = New-Object System.IO.MemoryStream
$img.Save($ms, $codec, $params)
$img.Dispose()
$b64 = [Convert]::ToBase64String($ms.ToArray())
Write-Output "===IMG-BEGIN $Name==="
for ($i = 0; $i -lt $b64.Length; $i += 1000) {
    Write-Output $b64.Substring($i, [Math]::Min(1000, $b64.Length - $i))
}
Write-Output "===IMG-END $Name==="
