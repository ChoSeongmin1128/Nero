// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "Grove",
    platforms: [.macOS(.v26)],
    products: [
        .library(name: "GroveInference", targets: ["GroveInference"]),
        .executable(name: "grove-apple-benchmark", targets: ["AppleBenchmark"]),
        .executable(name: "grove-diarization-benchmark", targets: ["DiarizationBenchmark"]),
        .executable(name: "Grove", targets: ["GroveApp"]),
    ],
    targets: [
        .executableTarget(name: "AppleBenchmark"),
        .target(name: "GroveInference"),
        .executableTarget(name: "DiarizationBenchmark", dependencies: ["GroveInference"]),
        .executableTarget(name: "GroveApp", dependencies: ["GroveInference"], resources: [.copy("Resources/Fonts"), .copy("Resources/Models")]),
        .testTarget(name: "GroveInferenceTests", dependencies: ["GroveInference"]),
        .testTarget(name: "GroveAppTests", dependencies: ["GroveApp"]),
    ]
)
