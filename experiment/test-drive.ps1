# dsh-desktop experiment test drive (loopback control plane).
# Proves: agent/CDP drives the exact WebView2 the human sees.
# Run while dsh-desktop is up:  pwsh -File experiment/test-drive.ps1
$base = "http://127.0.0.1:45331"
function Call($method, $path, $body) {
  $url = "$base$path"
  if ($method -eq "GET") { return Invoke-RestMethod -Uri $url -Method Get -TimeoutSec 15 }
  $json = $body | ConvertTo-Json -Compress -Depth 10
  return Invoke-RestMethod -Uri $url -Method Post -ContentType "application/json" -Body $json -TimeoutSec 20
}
Write-Output "== health =="
Call GET /health | ConvertTo-Json -Compress
Write-Output "== initial state =="
Call GET /state | ConvertTo-Json -Compress
Write-Output "== agent navigate -> example.com =="
Call POST /navigate @{ url = "https://example.com/" } | ConvertTo-Json -Compress
Start-Sleep -Seconds 3
Write-Output "== state after navigate =="
Call GET /state | ConvertTo-Json -Compress
Write-Output "== eval title =="
Call POST /eval @{ js = "document.title" } | ConvertTo-Json -Compress
Write-Output "== CDP Runtime.evaluate (same target proof) =="
Call POST /cdp @{ method = "Runtime.evaluate"; params = @{ expression = "location.href" } } | ConvertTo-Json -Compress
Write-Output "== CDP screenshot -> experiment/shot.png =="
$shot = Call POST /cdp @{ method = "Page.captureScreenshot"; params = @{ format = "png"; fromSurface = $true } }
$data = $shot.result.data
if ($data) {
  [IO.File]::WriteAllBytes("$PSScriptRoot/shot.png", [Convert]::FromBase64String($data))
  Write-Output "saved $($data.Length) chars -> experiment/shot.png"
} else {
  Write-Output "NO IMAGE DATA:"
  $shot | ConvertTo-Json -Compress
}
Write-Output "== overlay show (fake panel rect) =="
Call POST /rect @{ visible = $true; x = 900; y = 80; width = 460; height = 700; dpr = 1 } | ConvertTo-Json -Compress
Write-Output "== overlay hide =="
Call POST /rect @{ visible = $false; x = 0; y = 0; width = 0; height = 0; dpr = 1 } | ConvertTo-Json -Compress
Write-Output "DONE"
