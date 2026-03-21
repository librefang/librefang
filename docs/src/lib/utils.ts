export const getPathPrefix = () => {
	return "";
};

export const withPrefix = (path: string) => `${getPathPrefix()}${path}`;
