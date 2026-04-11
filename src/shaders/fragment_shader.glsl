#version 430 core
layout(std430, binding = 0) buffer GradientColors {
    int gradient_colors_size;
    vec4 gradient_colors[];
};
uniform vec2 WindowSize;
out vec4 fragColor;
void main() {
    if (gradient_colors_size == 1) {
        fragColor = gradient_colors[0];
    } else {
        float findex = (gl_FragCoord.y * float(gradient_colors_size - 1)) / WindowSize.y;
        int index = int(findex);
        float step = findex - float(index);
        if (index == gradient_colors_size - 1) {
            index--;
        }
        fragColor = mix(gradient_colors[index], gradient_colors[index + 1], step);
    }
}
