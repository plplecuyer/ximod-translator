# Reconstruire une traduction publiée (XIMOD Translator, ximod-cli, D-06),
# version PowerShell de rebuild.sh : apply, check, export DSD et override,
# manifest, package. Rien n'est écrit si le plugin manque (code 2).
#
# Usage : .\rebuild.ps1 -Plugin <plugin> -Out <dossier> [-Name <nom>]
#         [-Version <version>] [-Author <auteur>] [-Cli <ximod-cli.exe>]
#         [-Common @("--game","SkyrimSE","--to","fra")] [-Strict]
# -Strict : les réserves de check (erreurs trouvées) comptent dans le code.
param(
  [Parameter(Mandatory = $true)][string]$Plugin,
  [Parameter(Mandatory = $true)][string]$Out,
  [string]$Name = [System.IO.Path]::GetFileNameWithoutExtension($Plugin),
  [string]$Version = "1.0",
  [string]$Author = "",
  [string]$Cli = "ximod-cli.exe",
  [string[]]$Common = @(),
  [switch]$Strict
)
$script:code = 0
function Step {
  & $Cli @Common @args
  $rc = $LASTEXITCODE
  if ($rc -ge 2) { exit $rc }
  if ($rc -gt $script:code) { $script:code = $rc }
}
Step apply $Plugin
New-Item -ItemType Directory -Force $Out | Out-Null
& $Cli @Common check $Plugin --report "$Out.check.csv"
if ($LASTEXITCODE -ge 2) { exit $LASTEXITCODE }
if ($LASTEXITCODE -eq 1 -and $Strict) { $script:code = 1 }
Step export $Plugin --output dsd --statuses validated,pending -o $Out
Step export $Plugin --output override --statuses validated,pending -o $Out
Step manifest $Out --input $Plugin
Step package $Out --name $Name --version $Version --author $Author -o "$Out.zip"
exit $script:code
