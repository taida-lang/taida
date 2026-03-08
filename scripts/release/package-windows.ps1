param(
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,
    [Parameter(Mandatory = $true)]
    [string]$Tag,
    [Parameter(Mandatory = $true)]
    [string]$Target,
    [Parameter(Mandatory = $true)]
    [string]$OutDir
)

$ErrorActionPreference = "Stop"

$archiveBase = "taida-$Tag-$Target"
$stageRoot = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
$stageDir = Join-Path $stageRoot $archiveBase

New-Item -ItemType Directory -Path $stageDir -Force | Out-Null
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

Copy-Item $BinaryPath (Join-Path $stageDir "taida.exe")
Copy-Item "README.md" (Join-Path $stageDir "README.md")
Copy-Item "PHILOSOPHY.md" (Join-Path $stageDir "PHILOSOPHY.md")

$archivePath = Join-Path $OutDir "$archiveBase.zip"
Compress-Archive -Path (Join-Path $stageDir "*") -DestinationPath $archivePath -Force
Remove-Item -Path $stageRoot -Recurse -Force
