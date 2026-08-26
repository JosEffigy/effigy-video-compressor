Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing

$projectRoot = Split-Path -Parent $PSScriptRoot
$assetDir = Join-Path $projectRoot 'assets\itch'
$backgroundPath = Join-Path $assetDir 'abstract-bg.png'
$modernPath = Join-Path $assetDir 'current-ui.png'
$simplePath = Join-Path $assetDir 'current-simple-ui.png'

function New-RoundedPath {
    param([System.Drawing.RectangleF]$Rect, [float]$Radius)

    $diameter = $Radius * 2
    $path = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $path.AddArc($Rect.X, $Rect.Y, $diameter, $diameter, 180, 90)
    $path.AddArc($Rect.Right - $diameter, $Rect.Y, $diameter, $diameter, 270, 90)
    $path.AddArc($Rect.Right - $diameter, $Rect.Bottom - $diameter, $diameter, $diameter, 0, 90)
    $path.AddArc($Rect.X, $Rect.Bottom - $diameter, $diameter, $diameter, 90, 90)
    $path.CloseFigure()
    return $path
}

function New-Canvas {
    param([int]$Width, [int]$Height)

    $bitmap = [System.Drawing.Bitmap]::new($Width, $Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    return @{ Bitmap = $bitmap; Graphics = $graphics }
}

function Draw-Background {
    param(
        [System.Drawing.Graphics]$Graphics,
        [int]$Width,
        [int]$Height,
        [System.Drawing.Image]$Background
    )

    $sourceRatio = $Background.Width / $Background.Height
    $targetRatio = $Width / $Height
    if ($sourceRatio -gt $targetRatio) {
        $sourceHeight = $Background.Height
        $sourceWidth = [int]($sourceHeight * $targetRatio)
        $sourceX = [int](($Background.Width - $sourceWidth) / 2)
        $sourceY = 0
    } else {
        $sourceWidth = $Background.Width
        $sourceHeight = [int]($sourceWidth / $targetRatio)
        $sourceX = 0
        $sourceY = [int](($Background.Height - $sourceHeight) / 2)
    }

    $destination = [System.Drawing.Rectangle]::new(0, 0, $Width, $Height)
    $source = [System.Drawing.Rectangle]::new($sourceX, $sourceY, $sourceWidth, $sourceHeight)
    $Graphics.DrawImage($Background, $destination, $source, [System.Drawing.GraphicsUnit]::Pixel)

    $overlay = [System.Drawing.Drawing2D.LinearGradientBrush]::new(
        [System.Drawing.Point]::new(0, 0),
        [System.Drawing.Point]::new($Width, $Height),
        [System.Drawing.Color]::FromArgb(185, 8, 8, 12),
        [System.Drawing.Color]::FromArgb(55, 30, 12, 22)
    )
    $Graphics.FillRectangle($overlay, $destination)
    $overlay.Dispose()
}

function Draw-ScreenshotCard {
    param(
        [System.Drawing.Graphics]$Graphics,
        [System.Drawing.Image]$Screenshot,
        [System.Drawing.RectangleF]$Rect
    )

    $shadowRect = [System.Drawing.RectangleF]::new($Rect.X + 10, $Rect.Y + 14, $Rect.Width, $Rect.Height)
    $shadowPath = New-RoundedPath $shadowRect 14
    $shadowBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(150, 0, 0, 0))
    $Graphics.FillPath($shadowBrush, $shadowPath)
    $shadowBrush.Dispose()
    $shadowPath.Dispose()

    $frameRect = [System.Drawing.RectangleF]::new($Rect.X - 4, $Rect.Y - 4, $Rect.Width + 8, $Rect.Height + 8)
    $framePath = New-RoundedPath $frameRect 14
    $frameBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(220, 251, 113, 133))
    $Graphics.FillPath($frameBrush, $framePath)
    $frameBrush.Dispose()
    $framePath.Dispose()

    $Graphics.DrawImage($Screenshot, $Rect)
}

function Draw-Badge {
    param(
        [System.Drawing.Graphics]$Graphics,
        [string]$Text,
        [float]$X,
        [float]$Y,
        [float]$Width
    )

    $rect = [System.Drawing.RectangleF]::new($X, $Y, $Width, 38)
    $path = New-RoundedPath $rect 12
    $brush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(48, 251, 113, 133))
    $pen = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(120, 251, 113, 133), 1.5)
    $Graphics.FillPath($brush, $path)
    $Graphics.DrawPath($pen, $path)
    $font = [System.Drawing.Font]::new('Segoe UI', 15, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
    $textBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255, 255, 218, 224))
    $format = [System.Drawing.StringFormat]::new()
    $format.Alignment = [System.Drawing.StringAlignment]::Center
    $format.LineAlignment = [System.Drawing.StringAlignment]::Center
    $Graphics.DrawString($Text, $font, $textBrush, $rect, $format)
    $format.Dispose()
    $textBrush.Dispose()
    $font.Dispose()
    $pen.Dispose()
    $brush.Dispose()
    $path.Dispose()
}

function Save-Canvas {
    param($Canvas, [string]$Path)
    $Canvas.Bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $Canvas.Graphics.Dispose()
    $Canvas.Bitmap.Dispose()
}

$background = [System.Drawing.Image]::FromFile($backgroundPath)
$modern = [System.Drawing.Image]::FromFile($modernPath)
$simple = [System.Drawing.Image]::FromFile($simplePath)
$white = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(250, 248, 249, 252))
$muted = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(220, 189, 190, 201))
$rose = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255, 251, 113, 133))

try {
    # Store cover: 630 x 500.
    $cover = New-Canvas 630 500
    Draw-Background $cover.Graphics 630 500 $background
    $cover.Graphics.DrawString('EFFIGY', [System.Drawing.Font]::new('Segoe UI', 20, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel), $rose, 36, 27)
    $cover.Graphics.DrawString('VIDEO COMPRESSOR', [System.Drawing.Font]::new('Segoe UI', 34, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel), $white, 34, 50)
    $cover.Graphics.DrawString('Hit your target size. Keep the quality.', [System.Drawing.Font]::new('Segoe UI', 17, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel), $muted, 37, 101)
    Draw-Badge $cover.Graphics 'STRICT SIZE' 37 137 126
    Draw-Badge $cover.Graphics 'GPU READY' 174 137 120
    Draw-Badge $cover.Graphics 'PRIVATE' 305 137 105
    Draw-ScreenshotCard $cover.Graphics $modern ([System.Drawing.RectangleF]::new(37, 191, 556, 313))
    Save-Canvas $cover (Join-Path $assetDir 'cover-630x500.png')

    # Feature banner: strict target-size workflow.
    $target = New-Canvas 1280 720
    Draw-Background $target.Graphics 1280 720 $background
    $target.Graphics.DrawString('HIT THE SIZE LIMIT', [System.Drawing.Font]::new('Segoe UI', 48, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel), $white, 60, 83)
    $target.Graphics.DrawString("Two-pass targeting with adaptive audio`nand automatic final-size correction.", [System.Drawing.Font]::new('Segoe UI', 23, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel), $muted, 63, 153)
    Draw-Badge $target.Graphics 'FIXED MB' 63 251 120
    Draw-Badge $target.Graphics 'AUTO RES + FPS' 195 251 155
    Draw-Badge $target.Graphics 'MOTION AWARE' 362 251 150
    $target.Graphics.DrawString('Built for Discord limits, sharing, and repeatable results.', [System.Drawing.Font]::new('Segoe UI', 18, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel), $rose, 64, 316)
    Draw-ScreenshotCard $target.Graphics $modern ([System.Drawing.RectangleF]::new(535, 142, 700, 394))
    Save-Canvas $target (Join-Path $assetDir 'banner-target-size.png')

    # Feature banner: encoder support.
    $hardware = New-Canvas 1280 720
    Draw-Background $hardware.Graphics 1280 720 $background
    Draw-ScreenshotCard $hardware.Graphics $modern ([System.Drawing.RectangleF]::new(45, 155, 720, 405))
    $hardware.Graphics.DrawString("USE THE ENCODER`nYOU ALREADY HAVE", [System.Drawing.Font]::new('Segoe UI', 38, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel), $white, 825, 92)
    $hardware.Graphics.DrawString("Hardware options appear only when`nthey initialize successfully.", [System.Drawing.Font]::new('Segoe UI', 21, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel), $muted, 829, 224)
    Draw-Badge $hardware.Graphics 'NVENC' 829 343 110
    Draw-Badge $hardware.Graphics 'AMF' 951 343 90
    Draw-Badge $hardware.Graphics 'QSV' 1053 343 90
    Draw-Badge $hardware.Graphics 'x264 / x265' 829 397 145
    Draw-Badge $hardware.Graphics 'SVT-AV1' 986 397 130
    $hardware.Graphics.DrawString('Fast when you need it. Efficient when size matters.', [System.Drawing.Font]::new('Segoe UI', 17, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel), $rose, 830, 475)
    Save-Canvas $hardware (Join-Path $assetDir 'banner-encoders.png')

    # Feature banner: both interface themes.
    $themes = New-Canvas 1280 720
    Draw-Background $themes.Graphics 1280 720 $background
    $themes.Graphics.DrawString('YOUR WORKFLOW, YOUR LOOK', [System.Drawing.Font]::new('Segoe UI', 44, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel), $white, 60, 60)
    $themes.Graphics.DrawString('Modern for guided control. Simple for direct access.', [System.Drawing.Font]::new('Segoe UI', 22, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel), $muted, 63, 120)
    $themes.Graphics.DrawString('MODERN', [System.Drawing.Font]::new('Segoe UI', 17, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel), $rose, 63, 192)
    $themes.Graphics.DrawString('SIMPLE', [System.Drawing.Font]::new('Segoe UI', 17, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel), $rose, 664, 192)
    Draw-ScreenshotCard $themes.Graphics $modern ([System.Drawing.RectangleF]::new(63, 230, 552, 311))
    Draw-ScreenshotCard $themes.Graphics $simple ([System.Drawing.RectangleF]::new(664, 230, 552, 311))
    $themes.Graphics.DrawString('Dark, light, system, and custom accent colors included.', [System.Drawing.Font]::new('Segoe UI', 18, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel), $muted, 63, 592)
    Save-Canvas $themes (Join-Path $assetDir 'banner-themes.png')
}
finally {
    $rose.Dispose()
    $muted.Dispose()
    $white.Dispose()
    $simple.Dispose()
    $modern.Dispose()
    $background.Dispose()
}
