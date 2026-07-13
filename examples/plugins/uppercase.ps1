$context = [Console]::In.ReadToEnd() | ConvertFrom-Json

if ($null -eq $context.selection) {
    $response = @{ message = "Select text before running :plugin uppercase" }
}
else {
    $response = @{
        replace_selection = $context.selection.ToUpperInvariant()
        message = "Uppercased selection"
    }
}

$response | ConvertTo-Json -Compress
